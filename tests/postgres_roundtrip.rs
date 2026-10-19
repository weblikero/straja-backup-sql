//! Isolated real PostgreSQL test. Creates its own cluster on a free localhost port.
use anyhow::{Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use straja_sql::{engine, model::*};
fn call(bin: &Path, args: &[&str]) -> Result<()> {
    // pg_ctl's server child can inherit pipe handles, so never wait for EOF on its stdout.
    let log = tempfile::NamedTempFile::new()?;
    let status = Command::new(bin)
        .args(args)
        .stdout(log.reopen()?)
        .stderr(log.reopen()?)
        .status()?;
    ensure!(status.success(), "{}", fs::read_to_string(log.path())?);
    Ok(())
}
struct Server {
    bin: PathBuf,
    data: PathBuf,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = Command::new(self.bin.join("pg_ctl.exe"))
            .args(["stop", "-m", "immediate", "-D"])
            .arg(&self.data)
            .output();
    }
}
#[test]
#[ignore = "Requires PostgreSQL binaries; isolated temporary cluster only"]
fn real_export_compress_verify_restore_and_source_unchanged() -> Result<()> {
    let bin = PathBuf::from(
        std::env::var("STRAJA_TEST_PG_BIN").unwrap_or("C:/Program Files/PostgreSQL/17/bin".into()),
    );
    let temp = tempfile::tempdir()?;
    let data = temp.path().join("pgdata");
    let socket = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = socket.local_addr()?.port();
    drop(socket);
    call(
        &bin.join("initdb.exe"),
        &[
            "-D",
            data.to_str().unwrap(),
            "-U",
            "test_admin",
            "--auth=trust",
            "--encoding=UTF8",
            "--no-locale",
        ],
    )?;
    fs::write(
        data.join("pg_hba.conf"),
        "host all all 127.0.0.1/32 trust\n",
    )?;
    let options = format!("-p {port} -h 127.0.0.1");
    call(
        &bin.join("pg_ctl.exe"),
        &[
            "start",
            "-w",
            "-D",
            data.to_str().unwrap(),
            "-l",
            temp.path().join("server.log").to_str().unwrap(),
            "-o",
            &options,
        ],
    )?;
    let _server = Server {
        bin: bin.clone(),
        data,
    };
    let sql = |db: &str, statement: &str| -> Result<String> {
        let out = Command::new(bin.join("psql.exe"))
            .args([
                "-X",
                "-h",
                "127.0.0.1",
                "-p",
                &port.to_string(),
                "-U",
                "test_admin",
                "-d",
                db,
                "-v",
                "ON_ERROR_STOP=1",
                "-Atc",
                statement,
            ])
            .output()?;
        ensure!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(String::from_utf8_lossy(&out.stdout).trim().into())
    };
    sql("postgres", "CREATE DATABASE straja_source")?;
    sql("postgres", "CREATE DATABASE straja_restore")?;
    sql(
        "straja_source",
        "CREATE TABLE orders(id int PRIMARY KEY, note text); INSERT INTO orders SELECT n, 'Comand' || chr(259) || ' ' || n FROM generate_series(1,200) n; CREATE ROLE backup_reader LOGIN; GRANT CONNECT ON DATABASE straja_source TO backup_reader; GRANT USAGE ON SCHEMA public TO backup_reader; GRANT SELECT ON ALL TABLES IN SCHEMA public TO backup_reader;",
    )?;
    let c = Connection {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Isolated test".into(),
        engine: Engine::PostgreSql,
        host: "127.0.0.1".into(),
        port,
        user: "backup_reader".into(),
        tools: bin.clone(),
        tls: false,
        ca: String::new(),
    };
    let p = Plan {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Test".into(),
        connection: c.id.clone(),
        databases: vec!["straja_source".into()],
        destination: temp.path().into(),
        remote: None,
        schedule: Schedule::default(),
        compression: Compression::Gzip,
        keep_count: Some(1),
        keep_days: None,
        enabled: true,
        next_due: None,
        notify: false,
        timeout_minutes: 1,
    };
    assert!(
        engine::databases(&c, "", &temp.path().join("query"))?.contains(&"straja_source".into())
    );
    let folder = engine::backup(&p, &c, "", &uuid::Uuid::new_v4().to_string())?;
    let m = engine::verify(&folder)?;
    let restored = temp.path().join("restore.sql");
    std::io::copy(
        &mut flate2::read::GzDecoder::new(fs::File::open(folder.join(&m.files[0].file))?),
        &mut fs::File::create(&restored)?,
    )?;
    call(
        &bin.join("psql.exe"),
        &[
            "-X",
            "-h",
            "127.0.0.1",
            "-p",
            &port.to_string(),
            "-U",
            "test_admin",
            "-d",
            "straja_restore",
            "-v",
            "ON_ERROR_STOP=1",
            "-f",
            restored.to_str().unwrap(),
        ],
    )?;
    assert_eq!(sql("straja_restore", "SELECT count(*) FROM orders")?, "200");
    assert_eq!(
        sql(
            "straja_source",
            "SELECT md5(string_agg(id::text || note, ',' ORDER BY id)) FROM orders"
        )?,
        sql(
            "straja_restore",
            "SELECT md5(string_agg(id::text || note, ',' ORDER BY id)) FROM orders"
        )?
    );
    let mut broken = p.clone();
    broken.databases = vec!["missing_database".into()];
    assert!(engine::backup(&broken, &c, "", &uuid::Uuid::new_v4().to_string()).is_err());
    assert!(folder.exists());
    Ok(())
}
