//! Product-scoped remote backups. Secrets exist only in DPAPI storage and child memory.
//! No sync/purge/move commands; retention only touches verified files in owned receipts.
use crate::{
    engine,
    i18n::tr,
    model::{BackupProgress, Plan},
    store::Store,
};
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt, fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    Local,
    Ftp,
    GoogleDrive,
    Dropbox,
    S3,
}
impl Provider {
    pub const ALL: [Self; 5] = [
        Self::Local,
        Self::Ftp,
        Self::GoogleDrive,
        Self::Dropbox,
        Self::S3,
    ];
    fn backend(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Ftp => "ftp",
            Self::GoogleDrive => "drive",
            Self::Dropbox => "dropbox",
            Self::S3 => "s3",
        }
    }
    pub fn oauth(self) -> bool {
        matches!(self, Self::GoogleDrive | Self::Dropbox)
    }
}
impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Local => tr("Local folder / network share"),
            Self::Ftp => "FTP / FTPS",
            Self::GoogleDrive => "Google Drive",
            Self::Dropbox => "Dropbox",
            Self::S3 => "Amazon S3",
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Remote {
    pub id: String,
    pub provider: Provider,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub path: String,
    pub region: String,
    pub bucket: String,
    pub tls: bool,
    pub bandwidth_mib: u32,
}
impl Default for Remote {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            provider: Provider::Local,
            host: String::new(),
            port: 21,
            user: String::new(),
            path: "Backups".into(),
            region: "eu-central-1".into(),
            bucket: String::new(),
            tls: true,
            bandwidth_mib: 0,
        }
    }
}
impl Remote {
    pub fn secret_key(&self) -> String {
        format!("destination-{}", self.id)
    }
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.id)?;
        ensure!(
            self.provider != Provider::Local && self.bandwidth_mib <= 100000,
            tr("Invalid destination settings.")
        );
        for value in [
            &self.host,
            &self.user,
            &self.path,
            &self.region,
            &self.bucket,
        ] {
            ensure!(
                value.len() <= 1024 && !value.chars().any(char::is_control),
                tr("Invalid destination settings.")
            );
        }
        ensure!(
            !self.path.contains(['\\', ':'])
                && !self.path.starts_with('/')
                && self.path.split('/').all(|s| !matches!(s, "." | "..")),
            tr("Use a relative remote folder without .. or a drive letter.")
        );
        if self.provider == Provider::Ftp {
            ensure!(
                !self.host.is_empty()
                    && !self.host.contains(['/', '\\', '@', ' ', ':'])
                    && self.port > 0
                    && !self.user.is_empty(),
                tr("Invalid destination settings.")
            );
        }
        if self.provider == Provider::S3 {
            ensure!(
                (3..=63).contains(&self.bucket.len())
                    && self.bucket.bytes().all(|b| b.is_ascii_lowercase()
                        || b.is_ascii_digit()
                        || b == b'-'
                        || b == b'.')
                    && !self.bucket.contains("..")
                    && !self.user.is_empty()
                    && !self.region.is_empty()
                    && self
                        .region
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                tr("Invalid destination settings.")
            );
        }
        Ok(())
    }
    pub fn validate_secret(&self, value: &str) -> Result<()> {
        ensure!(
            !value.is_empty() && value.len() < 32768 && !value.contains(['\n', '\r', '\0']),
            tr("Enter destination credentials or connect your account.")
        );
        if self.provider.oauth() {
            let token: serde_json::Value = serde_json::from_str(value)?;
            ensure!(
                token["refresh_token"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()),
                tr("Connect your account again to allow scheduled backups.")
            );
        }
        Ok(())
    }
    fn fingerprint(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&Remote {
                    bandwidth_mib: 0,
                    ..self.clone()
                })
                .expect("remote serialization")
            )
        )
    }
    fn base(&self, plan: &str) -> Result<String> {
        self.validate()?;
        uuid::Uuid::parse_str(plan)?;
        let prefix = if self.provider == Provider::S3 {
            format!("{}/", self.bucket)
        } else {
            String::new()
        };
        let folder = self.path.trim_matches('/');
        Ok(format!(
            "target:{prefix}{}straja-plan-{plan}/{}",
            if folder.is_empty() {
                String::new()
            } else {
                format!("{folder}/")
            },
            self.id
        ))
    }
    pub fn label(&self) -> String {
        format!(
            "{} · {}{}",
            self.provider,
            if self.bucket.is_empty() {
                String::new()
            } else {
                format!("{}/", self.bucket)
            },
            self.path
        )
    }
}

pub fn bundled() -> Result<PathBuf> {
    let path = crate::platform::tools_root()?
        .join("transfer")
        .join(crate::platform::executable("rclone"));
    ensure!(
        path.is_file(),
        tr("The bundled transfer tool is missing. Reinstall the complete package.")
    );
    Ok(path)
}
fn base_command(binary: &Path) -> Command {
    let mut cmd = engine::command(binary);
    crate::platform::low_priority(&mut cmd);
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if name.starts_with("RCLONE_") || name.starts_with("AWS_") || name.starts_with("GOOGLE_") {
            cmd.env_remove(key);
        }
    }
    cmd.env("GOMAXPROCS", "2").env("GOMEMLIMIT", "128MiB");
    cmd.args([
        "--config",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
        "--transfers",
        "1",
        "--checkers",
        "1",
        "--buffer-size",
        "1M",
        "--max-buffer-memory",
        "32M",
        "--multi-thread-streams",
        "0",
        "--s3-upload-concurrency",
        "1",
        "--s3-chunk-size",
        "8M",
        "--drive-chunk-size",
        "8M",
        "--dropbox-chunk-size",
        "8M",
        "--retries",
        "2",
        "--low-level-retries",
        "2",
        "--contimeout",
        "20s",
        "--timeout",
        "90s",
        "--stats",
        "1s",
        "--stats-log-level",
        "NOTICE",
        "--use-json-log",
    ]);
    cmd
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Output {
    data: Zeroizing<Vec<u8>>,
    hash: String,
    bytes: u64,
}
fn run(
    cmd: &mut Command,
    input: Option<&str>,
    timeout: Duration,
    download: bool,
    max: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<Output> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = ChildGuard(
        cmd.spawn()
            .context(tr("Could not start the transfer tool."))?,
    );
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let abort = Arc::new(AtomicBool::new(false));
    let reader_abort = abort.clone();
    let (tx, rx) = mpsc::sync_channel(8);
    let out_tx = tx.clone();
    let reader = std::thread::spawn(move || -> Result<Output> {
        let mut stream = stdout;
        let mut buffer = [0u8; 65536];
        let mut data = Zeroizing::new(Vec::new());
        let mut hash = Sha256::new();
        let mut bytes = 0;
        let mut last = Instant::now();
        loop {
            let n = stream.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            if bytes > max {
                reader_abort.store(true, Ordering::Relaxed);
                bail!("Transfer output exceeds limit")
            }
            hash.update(&buffer[..n]);
            if !download {
                data.extend_from_slice(&buffer[..n]);
            }
            if download && last.elapsed() >= Duration::from_secs(1) {
                let _ = out_tx.try_send(bytes);
                last = Instant::now();
            }
        }
        if download {
            let _ = out_tx.try_send(bytes);
        }
        Ok(Output {
            data,
            hash: format!("{:x}", hash.finalize()),
            bytes,
        })
    });
    let errors = std::thread::spawn(move || {
        // Never persist raw stderr, which can contain OAuth URLs or provider tokens.
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if !download
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                && let Some(bytes) = value["stats"]["bytes"].as_u64()
            {
                let _ = tx.try_send(bytes);
            }
        }
    });
    if let Some(input) = input {
        let mut pipe = child.0.stdin.take().unwrap();
        pipe.write_all(input.as_bytes())?;
        pipe.write_all(b"\n")?;
    }
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        while let Ok(bytes) = rx.try_recv() {
            progress(bytes);
        }
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        if start.elapsed() > timeout || abort.load(Ordering::Relaxed) {
            timed_out = true;
            child.0.kill()?;
            break child.0.wait()?;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let output = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Transfer reader failed"))?;
    let _ = errors.join();
    while let Ok(bytes) = rx.try_recv() {
        progress(bytes);
    }
    ensure!(
        !timed_out,
        tr("Destination operation timed out or exceeded its output limit.")
    );
    ensure!(
        status.success(),
        "{} ({})",
        tr("Destination operation failed. Check credentials, permissions, free space and network."),
        status.code().unwrap_or(-1)
    );
    output
}
#[derive(Clone)]
pub struct Authorized(pub Zeroizing<String>);
impl fmt::Debug for Authorized {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Authorized([redacted])")
    }
}
pub fn authorize(provider: Provider) -> Result<Authorized> {
    ensure!(provider.oauth(), tr("Invalid destination settings."));
    let mut cmd = base_command(&bundled()?);
    cmd.args(["authorize", provider.backend()]);
    if provider == Provider::GoogleDrive {
        cmd.env("RCLONE_DRIVE_SCOPE", "drive.file");
    }
    let output = run(
        &mut cmd,
        None,
        Duration::from_secs(600),
        false,
        65536,
        &mut |_| {},
    )?;
    let text = std::str::from_utf8(&output.data)?;
    let token = text
        .split_once("--->")
        .and_then(|(_, s)| s.split_once("<---End paste"))
        .map(|(token, _)| token.trim())
        .context(tr("Account authorization did not return a token."))?;
    let remote = Remote {
        provider,
        ..Remote::default()
    };
    remote.validate_secret(token)?;
    Ok(Authorized(Zeroizing::new(token.into())))
}
struct Session {
    binary: PathBuf,
    remote: Remote,
    secret: Zeroizing<String>,
    deadline: Instant,
}
impl Session {
    fn new(binary: PathBuf, remote: &Remote, secret: &str, timeout: Duration) -> Result<Self> {
        remote.validate()?;
        remote.validate_secret(secret)?;
        let secret = if remote.provider == Provider::Ftp {
            let mut cmd = base_command(&binary);
            cmd.args(["obscure", "-"]);
            let output = run(
                &mut cmd,
                Some(secret),
                Duration::from_secs(10),
                false,
                65536,
                &mut |_| {},
            )?;
            Zeroizing::new(std::str::from_utf8(&output.data)?.trim().to_string())
        } else {
            Zeroizing::new(secret.into())
        };
        Ok(Self {
            binary,
            remote: remote.clone(),
            secret,
            deadline: Instant::now() + timeout,
        })
    }
    fn command(&self) -> Command {
        let r = &self.remote;
        let mut cmd = base_command(&self.binary);
        cmd.env("RCLONE_CONFIG_TARGET_TYPE", r.provider.backend());
        match r.provider {
            Provider::Ftp => {
                cmd.env("RCLONE_CONFIG_TARGET_HOST", &r.host)
                    .env("RCLONE_CONFIG_TARGET_PORT", r.port.to_string())
                    .env("RCLONE_CONFIG_TARGET_USER", &r.user)
                    .env("RCLONE_CONFIG_TARGET_PASS", self.secret.as_str())
                    .env("RCLONE_CONFIG_TARGET_EXPLICIT_TLS", r.tls.to_string())
                    .env("RCLONE_CONFIG_TARGET_TLS", "false")
                    .env("RCLONE_CONFIG_TARGET_NO_CHECK_CERTIFICATE", "false")
                    .env("RCLONE_CONFIG_TARGET_CONCURRENCY", "3");
            }
            Provider::GoogleDrive | Provider::Dropbox => {
                cmd.env("RCLONE_CONFIG_TARGET_TOKEN", self.secret.as_str());
                if r.provider == Provider::GoogleDrive {
                    cmd.env("RCLONE_CONFIG_TARGET_SCOPE", "drive.file");
                }
            }
            Provider::S3 => {
                cmd.env("RCLONE_CONFIG_TARGET_PROVIDER", "AWS")
                    .env("RCLONE_CONFIG_TARGET_ENV_AUTH", "false")
                    .env("RCLONE_CONFIG_TARGET_ACCESS_KEY_ID", &r.user)
                    .env(
                        "RCLONE_CONFIG_TARGET_SECRET_ACCESS_KEY",
                        self.secret.as_str(),
                    )
                    .env("RCLONE_CONFIG_TARGET_REGION", &r.region)
                    .env("RCLONE_CONFIG_TARGET_NO_CHECK_BUCKET", "true");
            }
            Provider::Local => {}
        }
        if r.bandwidth_mib > 0 {
            cmd.args(["--bwlimit", &format!("{}M", r.bandwidth_mib)]);
        }
        cmd
    }
    fn call(
        &self,
        args: &[&str],
        download: bool,
        max: u64,
        progress: &mut dyn FnMut(u64),
    ) -> Result<Output> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .context(tr(
                "Destination operation timed out or exceeded its output limit.",
            ))?;
        run(
            self.command().args(args),
            None,
            remaining,
            download,
            max,
            progress,
        )
    }
    fn upload(&self, local: &Path, remote: &str, progress: &mut dyn FnMut(u64)) -> Result<()> {
        self.call(
            &["copyto", &local.to_string_lossy(), remote, "--immutable"],
            false,
            65536,
            progress,
        )?;
        Ok(())
    }
    fn verify(
        &self,
        remote: &str,
        hash: &str,
        size: u64,
        progress: &mut dyn FnMut(u64),
    ) -> Result<()> {
        let output = self.call(&["cat", remote], true, size.saturating_add(1), progress)?;
        ensure!(
            output.bytes == size && output.hash == hash,
            tr("Remote backup integrity verification failed.")
        );
        Ok(())
    }
}
pub fn test(remote: &Remote, secret: &str) -> Result<()> {
    let session = Session::new(bundled()?, remote, secret, Duration::from_secs(60))?;
    // Read-only probe: S3 bucket or account root, not a folder that may not exist yet.
    let root = if remote.provider == Provider::S3 {
        format!("target:{}", remote.bucket)
    } else {
        "target:".into()
    };
    session.call(&["lsjson", &root, "--stat"], false, 65536, &mut |_| {})?;
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Receipt {
    fingerprint: String,
    manifest: engine::Manifest,
    manifest_hash: String,
    manifest_bytes: u64,
    deleting: bool,
}
fn receipt_key(plan: &str, run: &str, remote: &Remote) -> String {
    format!("{plan}:{}:{run}", remote.fingerprint())
}
fn report(plan: &Plan, phase: &str, index: usize, bytes: u64, total: u64) -> BackupProgress {
    BackupProgress {
        phase: phase.into(),
        database: plan.databases.get(index).cloned().unwrap_or_default(),
        database_index: index + 1,
        database_count: plan.databases.len(),
        bytes,
        total_bytes: Some(total),
        updated: Utc::now(),
    }
}
pub fn transfer(
    plan: &Plan,
    folder: &Path,
    store: &Store,
    progress: &mut dyn FnMut(BackupProgress),
) -> Result<()> {
    transfer_using(&bundled()?, plan, folder, store, progress)
}
fn transfer_using(
    binary: &Path,
    plan: &Plan,
    folder: &Path,
    store: &Store,
    progress: &mut dyn FnMut(BackupProgress),
) -> Result<()> {
    let remote = plan
        .remote
        .as_ref()
        .context(tr("Invalid destination settings."))?;
    let manifest = engine::verify(folder)?;
    ensure!(manifest.plan_id == plan.id, tr("Invalid manifest."));
    let secret = store.secret(&remote.secret_key())?;
    let session = Session::new(
        binary.into(),
        remote,
        &secret,
        Duration::from_secs(plan.timeout_minutes * 60),
    )?;
    let base = format!("{}/backup-{}", remote.base(&plan.id)?, manifest.run_id);
    let total = manifest.files.iter().map(|f| f.bytes).sum::<u64>();
    let mut done = 0;
    for (i, file) in manifest.files.iter().enumerate() {
        progress(report(plan, "Uploading backup", i, done, total));
        session.upload(
            &folder.join(&file.file),
            &format!("{base}/{}", file.file),
            &mut |n| {
                progress(report(
                    plan,
                    "Uploading backup",
                    i,
                    done + n.min(file.bytes),
                    total,
                ))
            },
        )?;
        done += file.bytes;
    }
    done = 0;
    for (i, file) in manifest.files.iter().enumerate() {
        progress(report(plan, "Verifying remote backup", i, done, total));
        session.verify(
            &format!("{base}/{}", file.file),
            &file.sha256,
            file.bytes,
            &mut |n| {
                progress(report(
                    plan,
                    "Verifying remote backup",
                    i,
                    done + n.min(file.bytes),
                    total,
                ))
            },
        )?;
        done += file.bytes;
    }
    // Publish the manifest only after every archive has been read back and verified.
    let bytes = fs::read(folder.join("manifest.json"))?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    session.upload(
        &folder.join("manifest.json"),
        &format!("{base}/manifest.json"),
        &mut |_| {},
    )?;
    session.verify(
        &format!("{base}/manifest.json"),
        &hash,
        bytes.len() as u64,
        &mut |_| {},
    )?;
    let receipt = Receipt {
        fingerprint: remote.fingerprint(),
        manifest_hash: hash,
        manifest_bytes: bytes.len() as u64,
        manifest,
        deleting: false,
    };
    store.put(
        "remote_receipt",
        &receipt_key(&plan.id, &receipt.manifest.run_id, remote),
        &receipt,
    )?;
    progress(report(plan, "Remote retention", 0, 0, 0));
    retain(plan, &session, store, &receipt.manifest.run_id, Utc::now())?;
    Ok(())
}
fn expired(p: &Plan, created: DateTime<Utc>, index: usize, now: DateTime<Utc>) -> bool {
    p.keep_count.is_some_and(|n| index >= n as usize)
        || p.keep_days
            .is_some_and(|n| created < now - chrono::Duration::days(n.into()))
}
fn retain(
    plan: &Plan,
    session: &Session,
    store: &Store,
    newest: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    if plan.keep_count.is_none() && plan.keep_days.is_none() {
        return Ok(());
    }
    let remote = &session.remote;
    let fingerprint = remote.fingerprint();
    let mut receipts: Vec<Receipt> = store
        .list::<Receipt>("remote_receipt")?
        .into_iter()
        .filter(|r| r.fingerprint == fingerprint && r.manifest.plan_id == plan.id)
        .collect();
    receipts.sort_by_key(|r| std::cmp::Reverse(r.manifest.created));
    for (index, mut r) in receipts.into_iter().enumerate() {
        if r.manifest.run_id == newest
            || (!r.deleting && !expired(plan, r.manifest.created, index, now))
        {
            continue;
        }
        uuid::Uuid::parse_str(&r.manifest.run_id)?;
        let names = engine::backup_filenames(
            &r.manifest
                .files
                .iter()
                .map(|f| f.database.clone())
                .collect::<Vec<_>>(),
        );
        ensure!(
            r.manifest.version == 2 && !r.manifest.files.is_empty(),
            tr("Invalid manifest.")
        );
        for (i, file) in r.manifest.files.iter().enumerate() {
            ensure!(
                file.file == format!("{}{}", names[i], if file.compressed { ".gz" } else { "" }),
                tr("Invalid manifest.")
            );
        }
        let base = format!("{}/backup-{}", remote.base(&plan.id)?, r.manifest.run_id);
        let listing = session.call(
            &["lsjson", &base, "--max-depth", "1"],
            false,
            1024 * 1024,
            &mut |_| {},
        )?;
        let listing: Vec<serde_json::Value> = serde_json::from_slice(&listing.data)?;
        let mut remaining = std::collections::HashSet::new();
        for entry in listing {
            let name = entry["Name"]
                .as_str()
                .context(tr("Remote retention refused unexpected files."))?;
            ensure!(
                !entry["IsDir"].as_bool().unwrap_or(true)
                    && (name == "manifest.json" || r.manifest.files.iter().any(|f| f.file == name)),
                tr("Remote retention refused unexpected files.")
            );
            ensure!(
                remaining.insert(name.to_string()),
                tr("Remote retention refused unexpected files.")
            );
        }
        if !r.deleting {
            ensure!(
                remaining.len() == r.manifest.files.len() + 1,
                tr("Remote retention refused unexpected files.")
            );
        }
        if remaining.contains("manifest.json") {
            session.verify(
                &format!("{base}/manifest.json"),
                &r.manifest_hash,
                r.manifest_bytes,
                &mut |_| {},
            )?;
        } else {
            ensure!(r.deleting, tr("Remote retention refused unexpected files."));
        }
        // Check every surviving file before any deletion, including retries after interruption.
        for file in &r.manifest.files {
            if remaining.contains(&file.file) {
                session.verify(
                    &format!("{base}/{}", file.file),
                    &file.sha256,
                    file.bytes,
                    &mut |_| {},
                )?;
            } else {
                ensure!(r.deleting, tr("Remote retention refused unexpected files."));
            }
        }
        r.deleting = true;
        let key = receipt_key(&plan.id, &r.manifest.run_id, remote);
        store.put("remote_receipt", &key, &r)?;
        for file in &r.manifest.files {
            if remaining.contains(&file.file) {
                session.call(
                    &["deletefile", &format!("{base}/{}", file.file)],
                    false,
                    65536,
                    &mut |_| {},
                )?;
            }
        }
        if remaining.contains("manifest.json") {
            session.call(
                &["deletefile", &format!("{base}/manifest.json")],
                false,
                65536,
                &mut |_| {},
            )?;
        }
        // rmdir only removes this exact empty folder; it is never recursive.
        session.call(&["rmdir", &base], false, 65536, &mut |_| {})?;
        store.delete("remote_receipt", &key)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Compression, Connection, Engine, Schedule};
    fn ftp() -> Remote {
        Remote {
            provider: Provider::Ftp,
            host: "127.0.0.1".into(),
            user: "fixture".into(),
            tls: false,
            ..Remote::default()
        }
    }
    fn fixture_plan(root: &Path) -> Plan {
        Plan {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Fixture".into(),
            connection: uuid::Uuid::new_v4().to_string(),
            databases: vec!["shop".into()],
            destination: root.into(),
            remote: Some(ftp()),
            schedule: Schedule::default(),
            compression: Compression::Gzip,
            keep_count: Some(1),
            keep_days: None,
            enabled: true,
            next_due: None,
            notify: false,
            timeout_minutes: 2,
        }
    }
    fn artifact(root: &Path, p: &Plan) -> PathBuf {
        let id = uuid::Uuid::new_v4().to_string();
        let folder = root.join(format!("backup-{id}"));
        fs::create_dir(&folder).unwrap();
        let file = folder.join("shop.sql");
        fs::write(&file, "SELECT 'fixture';\n".repeat(10000)).unwrap();
        let artifact = engine::archive(&file, true, "shop").unwrap();
        let manifest = engine::Manifest {
            version: 2,
            plan_id: p.id.clone(),
            run_id: id,
            created: Utc::now(),
            engine: Engine::MySql,
            files: vec![artifact],
        };
        fs::write(
            folder.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        folder
    }
    #[test]
    fn rejects_unsafe_paths_and_separates_product_accounts() {
        let p = uuid::Uuid::new_v4().to_string();
        let mut r = ftp();
        assert!(
            r.base(&p)
                .unwrap()
                .starts_with("target:Backups/straja-plan-")
        );
        for path in [
            "../other",
            "x/../other",
            "C:\\Users",
            "/root",
            "remote:other",
            "a\nb",
        ] {
            r.path = path.into();
            assert!(r.validate().is_err(), "{path}");
        }
        r = ftp();
        let fingerprint = r.fingerprint();
        r.bandwidth_mib = 3;
        assert_eq!(fingerprint, r.fingerprint());
        r.id = uuid::Uuid::new_v4().to_string();
        assert_ne!(fingerprint, r.fingerprint());
    }
    #[test]
    fn old_plans_default_to_local_and_credentials_are_encrypted() {
        let temp = tempfile::tempdir().unwrap();
        let p = fixture_plan(temp.path());
        let mut json = serde_json::to_value(&p).unwrap();
        json.as_object_mut().unwrap().remove("remote");
        let old: Plan = serde_json::from_value(json).unwrap();
        assert!(old.remote.is_none());
        let store = Store::open(temp.path()).unwrap();
        let r = p.remote.as_ref().unwrap();
        store.put_secret(&r.secret_key(), "fixture-secret").unwrap();
        let bytes: Vec<u8> = store
            .db
            .query_row("SELECT value FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert!(!bytes.windows(14).any(|w| w == b"fixture-secret"));
        assert_eq!(&*store.secret(&r.secret_key()).unwrap(), "fixture-secret");
    }
    #[test]
    fn oauth_requires_refresh_and_secret_never_enters_arguments() {
        let r = Remote {
            provider: Provider::S3,
            user: "key-id".into(),
            bucket: "test-bucket".into(),
            ..Remote::default()
        };
        let session = Session::new(
            PathBuf::from("unused.exe"),
            &r,
            "super-secret",
            Duration::from_secs(60),
        )
        .unwrap();
        let cmd = session.command();
        let args: Vec<_> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        assert!(!args.join(" ").contains("super-secret"));
        assert!(args.contains(&if cfg!(windows) {
            "NUL".into()
        } else {
            "/dev/null".into()
        }));
        assert!(args.contains(&"--transfers".into()));
        let r = Remote {
            provider: Provider::GoogleDrive,
            ..Remote::default()
        };
        assert!(
            r.validate_secret(r#"{"access_token":"short-lived"}"#)
                .is_err()
        );
        assert!(
            r.validate_secret(r#"{"refresh_token":"renewable"}"#)
                .is_ok()
        );
    }
    #[test]
    fn retention_limits_preserve_unlimited_and_use_either_limit() {
        let temp = tempfile::tempdir().unwrap();
        let mut p = fixture_plan(temp.path());
        let now = Utc::now();
        assert!(!expired(&p, now, 0, now));
        assert!(expired(&p, now, 1, now));
        p.keep_count = None;
        assert!(!expired(&p, now, 100, now));
        p.keep_days = Some(1);
        assert!(expired(&p, now - chrono::Duration::days(2), 0, now));
    }
    #[test]
    #[ignore = "Explicit loopback FTP fixture; requires bundled rclone and scripts/test_ftp_server.py dependencies"]
    fn native_ftp_upload_readback_retention_and_failure() {
        let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let binary = project.join("vendor-tools/transfer/rclone.exe");
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let ftp_root = root.join("remote");
        let local = root.join("local");
        fs::create_dir(&ftp_root).unwrap();
        fs::create_dir(&local).unwrap();
        let port_file = root.join("port");
        let python = std::env::var("STRAJA_TEST_PYTHON").unwrap_or("python".into());
        let server = engine::command(python)
            .arg(project.join("scripts/test_ftp_server.py"))
            .arg(&ftp_root)
            .arg(&port_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let _server = ChildGuard(server);
        let until = Instant::now() + Duration::from_secs(10);
        while !port_file.exists() {
            assert!(Instant::now() < until, "FTP fixture did not start");
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut p = fixture_plan(&local);
        p.remote.as_mut().unwrap().port = fs::read_to_string(&port_file).unwrap().parse().unwrap();
        let store = Store::open(&root.join("profile")).unwrap();
        let connection = Connection {
            id: p.connection.clone(),
            name: "Fixture".into(),
            engine: Engine::MySql,
            host: "never-connect".into(),
            port: 3306,
            user: "fixture".into(),
            tools: local.clone(),
            tls: true,
            ca: String::new(),
        };
        store
            .put("connection", &connection.id, &connection)
            .unwrap();
        store.save_plan_destination(&p, "fixture-secret").unwrap();
        let first = artifact(&local, &p);
        let mut phases = vec![];
        transfer_using(&binary, &p, &first, &store, &mut |p| phases.push(p.phase)).unwrap();
        assert!(phases.iter().any(|s| s == "Uploading backup"));
        assert!(phases.iter().any(|s| s == "Verifying remote backup"));
        let remote = p.remote.as_ref().unwrap();
        let base = ftp_root.join(remote.base(&p.id).unwrap().strip_prefix("target:").unwrap());
        let remote_first = base.join(first.file_name().unwrap());
        engine::verify(&remote_first).unwrap();
        fs::write(remote_first.join("foreign.txt"), "not owned").unwrap();
        std::thread::sleep(Duration::from_millis(5));
        let second = artifact(&local, &p);
        assert!(transfer_using(&binary, &p, &second, &store, &mut |_| {}).is_err());
        assert!(remote_first.join("foreign.txt").exists());
        assert!(remote_first.join("shop.sql.gz").exists());
        // Remove only the foreign file created by this fixture, then retry the same upload.
        fs::remove_file(remote_first.join("foreign.txt")).unwrap();
        transfer_using(&binary, &p, &second, &store, &mut |_| {}).unwrap();
        assert!(!remote_first.exists());
        engine::verify(&base.join(second.file_name().unwrap())).unwrap();
        assert!(first.exists());
        let session = Session::new(
            binary.clone(),
            remote,
            "fixture-secret",
            Duration::from_secs(30),
        )
        .unwrap();
        assert!(
            session
                .verify(
                    &format!(
                        "{}/{}/shop.sql.gz",
                        remote.base(&p.id).unwrap(),
                        second.file_name().unwrap().to_string_lossy()
                    ),
                    &"0".repeat(64),
                    1,
                    &mut |_| {}
                )
                .is_err()
        );
        let mut denied = p.clone();
        denied.remote.as_mut().unwrap().id = uuid::Uuid::new_v4().to_string();
        store
            .put_secret(
                &denied.remote.as_ref().unwrap().secret_key(),
                "wrong-password",
            )
            .unwrap();
        let third = artifact(&local, &denied);
        assert!(transfer_using(&binary, &denied, &third, &store, &mut |_| {}).is_err());
        engine::verify(&third).unwrap();
    }
}
