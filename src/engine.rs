use crate::i18n::{self, tr};
use crate::model::*;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration as Days, Utc};
use flate2::{read::GzDecoder, write::GzEncoder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::os::windows::{fs::MetadataExt, process::CommandExt};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn command(exe: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(exe);
    c.creation_flags(0x08000000);
    c.stdin(Stdio::null());
    c
}
// No shell, no user-supplied SQL or command-line options. Passwords are scoped to the child environment.
pub fn connection_command(c: &Connection, password: &str, dump: bool) -> Result<Command> {
    let tools = c.tools_dir()?;
    let mut cmd = command(tools.join(if dump {
        c.engine.dump()
    } else {
        c.engine.client()
    }));
    // Only our SQL client process gets reduced priority; server processes are untouched.
    cmd.creation_flags(
        0x08000000 | windows_sys::Win32::System::Threading::BELOW_NORMAL_PRIORITY_CLASS,
    );
    // App-local DLLs and resources are isolated from the caller's working directory.
    cmd.current_dir(&tools);
    if c.engine == Engine::PostgreSql {
        cmd.args([
            "--no-password",
            "--host",
            &c.host,
            "--port",
            &c.port.to_string(),
            "--username",
            &c.user,
        ]);
        cmd.env("PGPASSWORD", password)
            .env("PGSSLMODE", if c.tls { "verify-full" } else { "disable" })
            .env("PGCONNECT_TIMEOUT", "15")
            .env("PGCLIENTENCODING", "UTF8")
            .env("PGOPTIONS", "-c default_transaction_read_only=on")
            .env_remove("PGSERVICE")
            .env_remove("PGSERVICEFILE");
        if !c.ca.is_empty() {
            cmd.env("PGSSLROOTCERT", &c.ca);
        }
    } else {
        cmd.arg("--no-defaults");
        if c.engine == Engine::MySql {
            cmd.arg("--no-login-paths");
        }
        cmd.args([
            "--protocol=TCP",
            &format!("--host={}", c.host),
            &format!("--port={}", c.port),
            &format!("--user={}", c.user),
            "--default-character-set=utf8mb4",
        ]);
        if !dump {
            cmd.arg("--connect-timeout=15");
        }
        cmd.env("MYSQL_PWD", password);
        if c.tools.as_os_str().is_empty() {
            let root = tools.parent().context(tr("Invalid tools folder."))?;
            cmd.arg(format!(
                "--plugin-dir={}",
                root.join("lib/plugin").display()
            ));
            cmd.arg(format!(
                "--character-sets-dir={}",
                root.join("share/charsets").display()
            ));
        }
        if c.engine == Engine::MySql {
            cmd.arg(if c.tls {
                "--ssl-mode=VERIFY_IDENTITY"
            } else {
                "--ssl-mode=DISABLED"
            });
            if !c.tls {
                cmd.arg("--get-server-public-key");
            }
        } else if c.tls {
            cmd.args(["--ssl", "--ssl-verify-server-cert"]);
        } else {
            cmd.arg("--skip-ssl");
        }
        if !c.ca.is_empty() {
            cmd.arg(format!("--ssl-ca={}", c.ca));
        }
    }
    Ok(cmd)
}
fn run(cmd: &mut Command, output: &Path, timeout: Duration, password: &str) -> Result<()> {
    run_progress(cmd, output, timeout, password, &mut |_| {})
}
fn run_progress(
    cmd: &mut Command,
    output: &Path,
    timeout: Duration,
    password: &str,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    let err_path = output.with_extension("stderr");
    let stdout = File::options().create_new(true).write(true).open(output)?;
    let stderr = File::options()
        .create_new(true)
        .write(true)
        .open(&err_path)?;
    let spawn = cmd.stdout(stdout).stderr(stderr).spawn();
    let result = (|| -> Result<()> {
        let mut child = spawn.context(tr(
            "Could not start the SQL tool. Check its bin folder and DLLs.",
        ))?;
        let start = Instant::now();
        let mut last = Instant::now();
        progress(0);
        let status = loop {
            if last.elapsed() >= Duration::from_secs(1) {
                progress(fs::metadata(output).map(|m| m.len()).unwrap_or(0));
                last = Instant::now();
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e.into());
                }
                _ => {}
            }
            if start.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{}", tr("Time limit exceeded; export stopped."));
            }
            std::thread::sleep(Duration::from_millis(250));
        };
        progress(fs::metadata(output).map(|m| m.len()).unwrap_or(0));
        if !status.success() {
            let mut message = String::new();
            File::open(&err_path)?
                .take(16000)
                .read_to_string(&mut message)
                .ok();
            if !password.is_empty() {
                message = message.replace(password, tr("[password hidden]"));
            }
            bail!(
                "{}",
                i18n::format(
                    "SQL tool: {0} · {1}",
                    &[(status).to_string(), (message.trim()).to_string()]
                )
            );
        }
        Ok(())
    })();
    let _ = fs::remove_file(err_path);
    result
}

pub fn databases(c: &Connection, password: &str, temp: &Path) -> Result<Vec<String>> {
    c.validate()?;
    fs::create_dir_all(temp)?;
    let out = temp.join(format!("query-{}.txt", uuid::Uuid::new_v4()));
    let mut cmd = connection_command(c, password, false)?;
    if c.engine == Engine::PostgreSql {
        cmd.args(["-X", "--tuples-only", "--no-align", "--set=ON_ERROR_STOP=1", "--dbname=postgres", "--command=SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname"]);
    } else {
        cmd.args(["--batch", "--skip-column-names", "--execute=SHOW DATABASES"]);
    }
    let result = run(&mut cmd, &out, Duration::from_secs(30), password).and_then(|_| {
        let content = fs::read_to_string(&out)?;
        Ok(content
            .lines()
            .map(str::trim)
            .filter(|s| {
                !s.is_empty()
                    && !["information_schema", "performance_schema", "mysql", "sys"].contains(s)
            })
            .map(String::from)
            .collect())
    });
    let _ = fs::remove_file(out);
    result
}

pub fn tables_command(c: &Connection, password: &str, database: &str) -> Result<Command> {
    validate_database(database)?;
    let mut cmd = connection_command(c, password, false)?;
    if c.engine == Engine::PostgreSql {
        cmd.env("PGDATABASE", database).args(["-X", "--tuples-only", "--no-align", "--field-separator=\t", "--set=ON_ERROR_STOP=1",
            "--command=SELECT encode(convert_to(n.nspname,'UTF8'),'hex'), encode(convert_to(c.relname,'UTF8'),'hex'), c.relkind FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p','v','m','f') AND n.nspname <> 'information_schema' AND n.nspname !~ '^pg_' ORDER BY n.nspname,c.relname"]);
    } else {
        cmd.arg(format!("--database={database}")).args(["--batch", "--skip-column-names",
            "--execute=SELECT HEX(TABLE_SCHEMA), HEX(TABLE_NAME), TABLE_TYPE FROM information_schema.TABLES WHERE TABLE_SCHEMA=DATABASE() ORDER BY TABLE_NAME"]);
    }
    Ok(cmd)
}
pub fn parse_tables(content: &str) -> Result<Vec<TableInfo>> {
    fn decode(hex: &str) -> Result<String> {
        if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
            bail!("{}", tr("Invalid SQL metadata."));
        }
        let bytes = hex
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u8::from_str_radix(std::str::from_utf8(c)?, 16).map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()?;
        Ok(String::from_utf8(bytes)?)
    }
    content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() != 3 {
                bail!("{}", tr("Unexpected table response format."));
            }
            Ok(TableInfo {
                schema: decode(fields[0])?,
                name: decode(fields[1])?,
                kind: match fields[2] {
                    "r" | "BASE TABLE" => "Tabel",
                    "p" => "Tabel partiționat",
                    "v" | "VIEW" => "Vedere",
                    "m" => "Vedere materializată",
                    "f" => "Tabel extern",
                    other => other,
                }
                .into(),
            })
        })
        .collect()
}
pub fn tables(
    c: &Connection,
    password: &str,
    database: &str,
    temp: &Path,
) -> Result<Vec<TableInfo>> {
    c.validate()?;
    fs::create_dir_all(temp)?;
    let out = temp.join(format!("tables-{}.txt", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut cmd = tables_command(c, password, database)?;
        run(&mut cmd, &out, Duration::from_secs(30), password)?;
        parse_tables(&fs::read_to_string(&out)?)
    })();
    let _ = fs::remove_file(out);
    result
}

/// Preserve legal names; make Windows-unsafe names deterministic and collision-safe.
pub fn backup_filenames(databases: &[String]) -> Vec<String> {
    let stems = databases
        .iter()
        .map(|db| {
            let cleaned: String = db
                .chars()
                .map(|c| {
                    if c.is_control() || "<>:\"/\\|?*".contains(c) {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            let cleaned = cleaned.trim_matches([' ', '.']);
            let prefix = cleaned
                .split('.')
                .next()
                .unwrap_or("")
                .trim_end_matches(' ')
                .to_uppercase();
            let reserved = ["CON", "PRN", "AUX", "NUL", "CLOCK$"].contains(&prefix.as_str())
                || ["COM", "LPT"].iter().any(|p| {
                    prefix.strip_prefix(p).is_some_and(|s| {
                        ["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"].contains(&s)
                    })
                });
            if cleaned != db
                || reserved
                || cleaned.is_empty()
                || cleaned.encode_utf16().count() > 100
            {
                let digest = format!("{:x}", Sha256::digest(db.as_bytes()));
                let mut stem: String = cleaned.chars().take(60).collect();
                // CON.txt remains a Windows device even with a suffix after the dot.
                if reserved {
                    stem.insert(0, '_');
                }
                format!(
                    "{}--{}",
                    if stem.is_empty() { "database" } else { &stem },
                    &digest[..16]
                )
            } else {
                cleaned.into()
            }
        })
        .collect::<Vec<String>>();
    let mut used = std::collections::HashSet::new();
    stems
        .iter()
        .enumerate()
        .map(|(i, stem)| {
            let mut candidate = stem.clone();
            let mut suffix = 0;
            while !used.insert(candidate.to_lowercase()) {
                suffix += 1;
                candidate = format!("{stem}--{}-{suffix}", i + 1);
            }
            format!("{candidate}.sql")
        })
        .collect()
}

struct CountedReader<R> {
    inner: R,
    bytes: u64,
}
impl<R: Read> Read for CountedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes += n as u64;
        Ok(n)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub database: String,
    pub file: String,
    pub sha256: String,
    pub raw_sha256: String,
    pub bytes: u64,
    pub compressed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub plan_id: String,
    pub run_id: String,
    pub created: DateTime<Utc>,
    pub engine: Engine,
    pub files: Vec<Artifact>,
}
fn hash_reader(r: impl Read) -> Result<(String, u64)> {
    hash_reader_progress(r, &mut |_| {})
}
fn hash_reader_progress(mut r: impl Read, progress: &mut dyn FnMut(u64)) -> Result<(String, u64)> {
    let mut h = Sha256::new();
    let mut count = 0u64;
    let mut b = [0u8; 65536];
    let mut last = Instant::now();
    progress(0);
    loop {
        let n = r.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
        count += n as u64;
        if last.elapsed() >= Duration::from_secs(1) {
            progress(count);
            last = Instant::now();
        }
    }
    progress(count);
    Ok((format!("{:x}", h.finalize()), count))
}
pub fn hash(path: &Path) -> Result<(String, u64)> {
    hash_reader(File::open(path)?)
}
pub fn ordinary(path: &Path, directory: bool) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if m.file_attributes() & 0x400 != 0
        || (directory && !m.is_dir())
        || (!directory && !m.is_file())
    {
        bail!(
            "{}",
            i18n::format(
                "Links and reparse points are not supported: {0}",
                &[(path.display()).to_string()]
            )
        );
    }
    Ok(())
}
pub fn archive(raw: &Path, compressed: bool, database: &str) -> Result<Artifact> {
    archive_progress(raw, compressed, database, &mut |_, _, _| {})
}
fn archive_progress(
    raw: &Path,
    compressed: bool,
    database: &str,
    progress: &mut dyn FnMut(&str, u64, u64),
) -> Result<Artifact> {
    let raw_size = fs::metadata(raw)?.len();
    let (raw_sha256, size) = hash_reader_progress(File::open(raw)?, &mut |n| {
        progress("Verificare export", n, raw_size)
    })?;
    if size == 0 {
        bail!(
            "{}",
            i18n::format("Empty export for {0}.", &[(database).to_string()])
        );
    }
    let path = if compressed {
        let dest = raw.with_extension("sql.gz");
        let mut gz = GzEncoder::new(
            File::options().write(true).create_new(true).open(&dest)?,
            flate2::Compression::fast(),
        );
        let mut input = File::open(raw)?;
        let mut buffer = [0u8; 65536];
        let mut count = 0;
        let mut last = Instant::now();
        progress("Comprimare", 0, size);
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            gz.write_all(&buffer[..n])?;
            count += n as u64;
            if last.elapsed() >= Duration::from_secs(1) {
                progress("Comprimare", count, size);
                last = Instant::now();
            }
        }
        progress("Comprimare", count, size);
        gz.finish()?.sync_all()?;
        drop(input);
        if hash_reader_progress(GzDecoder::new(File::open(&dest)?), &mut |n| {
            progress("Verificare GZIP", n, size)
        })?
        .0 != raw_sha256
        {
            bail!("{}", tr("Archive does not match the export."));
        }
        fs::remove_file(raw)?;
        dest
    } else {
        raw.to_path_buf()
    };
    let archive_size = fs::metadata(&path)?.len();
    let (sha256, bytes) = hash_reader_progress(File::open(&path)?, &mut |n| {
        progress("Verificare fișier", n, archive_size)
    })?;
    Ok(Artifact {
        database: database.into(),
        file: path.file_name().unwrap().to_string_lossy().to_string(),
        sha256,
        raw_sha256,
        bytes,
        compressed,
    })
}
pub fn verify(folder: &Path) -> Result<Manifest> {
    verify_progress(folder, &mut |_, _, _, _| {})
}
fn verify_progress(
    folder: &Path,
    progress: &mut dyn FnMut(usize, &str, u64, u64),
) -> Result<Manifest> {
    ordinary(folder, true)?;
    ordinary(&folder.join("manifest.json"), false)?;
    let m: Manifest = serde_json::from_reader(File::open(folder.join("manifest.json"))?)?;
    uuid::Uuid::parse_str(&m.plan_id)?;
    uuid::Uuid::parse_str(&m.run_id)?;
    if ![1, 2].contains(&m.version) || m.files.is_empty() {
        bail!("{}", tr("Invalid manifest."));
    }
    let names = backup_filenames(
        &m.files
            .iter()
            .map(|a| a.database.clone())
            .collect::<Vec<_>>(),
    );
    for (index, a) in m.files.iter().enumerate() {
        let expected = if m.version == 2 {
            format!("{}{}", names[index], if a.compressed { ".gz" } else { "" })
        } else {
            format!(
                "database-{:03}.sql{}",
                index + 1,
                if a.compressed { ".gz" } else { "" }
            )
        };
        if a.file != expected {
            bail!("{}", tr("Unexpected file name in manifest."));
        }
        let p = folder.join(&a.file);
        ordinary(&p, false)?;
        let total = fs::metadata(&p)?.len();
        let (digest, size) = hash_reader_progress(File::open(&p)?, &mut |n| {
            progress(index, "Validare finală · fișier", n, total)
        })?;
        if digest != a.sha256 || size != a.bytes {
            bail!(
                "{}",
                i18n::format(
                    "Integrity check failed: {0}",
                    std::slice::from_ref(&a.database)
                )
            );
        }
        let raw_digest = if a.compressed {
            // GZIP trailer's ISIZE wraps at 4 GiB: count compressed input instead.
            let mut input = CountedReader {
                inner: File::open(&p)?,
                bytes: 0,
            };
            let mut decoder = flate2::bufread::GzDecoder::new(std::io::BufReader::new(&mut input));
            let mut hasher = Sha256::new();
            let mut buffer = [0u8; 65536];
            let mut last = Instant::now();
            progress(index, "Validare finală · conținut", 0, total);
            loop {
                let n = decoder.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
                if last.elapsed() >= Duration::from_secs(1) {
                    progress(
                        index,
                        "Validare finală · conținut",
                        decoder.get_ref().get_ref().bytes,
                        total,
                    );
                    last = Instant::now();
                }
            }
            progress(index, "Validare finală · conținut", total, total);
            format!("{:x}", hasher.finalize())
        } else {
            digest
        };
        if raw_digest != a.raw_sha256 {
            bail!(
                "{}",
                i18n::format("Invalid content: {0}", std::slice::from_ref(&a.database))
            );
        }
    }
    Ok(m)
}
pub fn backup(p: &Plan, c: &Connection, password: &str, run_id: &str) -> Result<PathBuf> {
    backup_with_progress(p, c, password, run_id, &mut |_| {})
}
pub fn backup_with_progress(
    p: &Plan,
    c: &Connection,
    password: &str,
    run_id: &str,
    progress: &mut dyn FnMut(BackupProgress),
) -> Result<PathBuf> {
    let mut report = |index: usize, phase: &str, bytes: u64, total_bytes: Option<u64>| {
        progress(BackupProgress {
            phase: phase.into(),
            database: p.databases.get(index).cloned().unwrap_or_default(),
            database_index: index + 1,
            database_count: p.databases.len(),
            bytes,
            total_bytes,
            updated: Utc::now(),
        })
    };
    p.validate()?;
    c.validate()?;
    uuid::Uuid::parse_str(run_id)?;
    ordinary(&p.destination, true)?;
    let root = fs::canonicalize(&p.destination)?;
    let plan_dir = root.join(format!("straja-plan-{}", p.id));
    fs::create_dir_all(&plan_dir)?;
    ordinary(&plan_dir, true)?;
    let staging = plan_dir.join(format!("partial-{run_id}"));
    fs::create_dir(&staging)?;
    let mut manifest = Manifest {
        version: 2,
        plan_id: p.id.clone(),
        run_id: run_id.into(),
        created: Utc::now(),
        engine: c.engine,
        files: vec![],
    };
    let filenames = backup_filenames(&p.databases);
    for (i, db) in p.databases.iter().enumerate() {
        validate_database(db)?;
        let raw = staging.join(&filenames[i]);
        let mut cmd = connection_command(c, password, true)?;
        if c.engine == Engine::PostgreSql {
            // PGDATABASE avoids libpq interpreting a database name containing '=' as a connection string.
            cmd.env("PGDATABASE", db).args([
                "--format=plain",
                "--no-password",
                "--lock-wait-timeout=30000",
            ]);
        } else {
            cmd.args([
                "--single-transaction",
                "--quick",
                "--skip-lock-tables",
                "--no-tablespaces",
                "--routines",
                "--events",
                "--triggers",
                "--hex-blob",
            ]);
            if c.engine == Engine::MySql {
                cmd.args(["--set-gtid-purged=OFF", "--column-statistics=0"]);
            }
            cmd.arg("--databases").arg(db);
        }
        run_progress(
            &mut cmd,
            &raw,
            Duration::from_secs(p.timeout_minutes * 60),
            password,
            &mut |n| report(i, "Export", n, None),
        )?;
        // Refuse an abnormal/truncated export even when the tool reported a zero exit status.
        let mut f = File::open(&raw)?;
        use std::io::{Seek, SeekFrom};
        let len = f.metadata()?.len();
        f.seek(SeekFrom::Start(len.saturating_sub(8192)))?;
        let mut tail_bytes = Vec::new();
        f.read_to_end(&mut tail_bytes)?;
        let tail = String::from_utf8_lossy(&tail_bytes);
        let marker = if c.engine == Engine::PostgreSql {
            "PostgreSQL database dump complete"
        } else {
            "Dump completed on"
        };
        if !tail.contains(marker) {
            bail!(
                "{}",
                i18n::format(
                    "Export {0} is missing the expected completion marker.",
                    std::slice::from_ref(db)
                )
            );
        }
        manifest.files.push(archive_progress(
            &raw,
            p.compression == Compression::Gzip,
            db,
            &mut |phase, n, total| report(i, phase, n, Some(total)),
        )?);
    }
    let mut file = File::options()
        .create_new(true)
        .write(true)
        .open(staging.join("manifest.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    file.sync_all()?;
    drop(file);
    verify_progress(&staging, &mut |i, phase, n, total| {
        report(i, phase, n, Some(total))
    })?;
    let final_path = plan_dir.join(format!("backup-{run_id}"));
    fs::rename(staging, &final_path)?;
    Ok(final_path)
}
pub fn retention(p: &Plan, newest: &Path, now: DateTime<Utc>) -> Result<usize> {
    if p.keep_count.is_none() && p.keep_days.is_none() {
        return Ok(0);
    }
    let parent = newest.parent().context(tr("Folder has no parent"))?;
    ordinary(parent, true)?;
    let expected = fs::canonicalize(&p.destination)?.join(format!("straja-plan-{}", p.id));
    if fs::canonicalize(parent)? != expected {
        bail!("{}", tr("Retention refused a folder outside this plan."));
    }
    let mut candidates = vec![];
    for entry in fs::read_dir(parent)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(id) = name.strip_prefix("backup-") else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err() {
            continue;
        }
        if let Ok(m) = verify(&path)
            && m.plan_id == p.id
            && m.run_id == id
        {
            candidates.push((path, m));
        }
    }
    candidates.sort_by_key(|a| std::cmp::Reverse(a.1.created));
    let mut deleted = 0;
    for (i, (path, m)) in candidates.into_iter().enumerate() {
        if path == newest {
            continue;
        }
        let expired = p.keep_count.is_some_and(|n| i >= n as usize)
            || p.keep_days
                .is_some_and(|n| m.created < now - Days::days(n.into()));
        if !expired {
            continue;
        }
        // Never recurse. Refuse folders with unrecognized files; delete only manifest-owned ordinary files.
        let entries = fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
        if entries.len() != m.files.len() + 1 {
            continue;
        }
        for a in &m.files {
            ordinary(&path.join(&a.file), false)?;
        }
        ordinary(&path.join("manifest.json"), false)?;
        for a in &m.files {
            fs::remove_file(path.join(&a.file))?;
        }
        fs::remove_file(path.join("manifest.json"))?;
        fs::remove_dir(path)?;
        deleted += 1;
    }
    Ok(deleted)
}
