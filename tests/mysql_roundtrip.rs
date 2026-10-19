use anyhow::{Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use straja_sql::{engine, model::*};
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn sql(bin: &Path, port: u16, statement: &str) -> Result<String> {
    let mut cmd = Command::new(bin);
    cmd.arg("--no-defaults");
    let out = cmd
        .args([
            "--protocol=TCP",
            "--host=127.0.0.1",
            &format!("--port={port}"),
            "--user=root",
            "--batch",
            "--skip-column-names",
            "--default-character-set=utf8mb4",
            "--execute",
            statement,
        ])
        .output()?;
    ensure!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8_lossy(&out.stdout).trim().into())
}
fn roundtrip(engine_kind: Engine, bin: PathBuf) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let data = temp.path().join("data");
    fs::create_dir(&data)?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let mut init = Command::new(bin.join(if engine_kind == Engine::MySql {
        "mysqld.exe"
    } else {
        "mariadb-install-db.exe"
    }));
    if engine_kind == Engine::MySql {
        init.args(["--no-defaults", "--initialize-insecure"]);
    }
    init.arg(format!("--datadir={}", data.display()));
    let output = init.output()?;
    ensure!(
        output.status.success(),
        "Init: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = fs::File::create(temp.path().join("server.log"))?;
    let child = Command::new(bin.join(if engine_kind == Engine::MySql {
        "mysqld.exe"
    } else {
        "mariadbd.exe"
    }))
    .args([
        "--no-defaults",
        &format!("--datadir={}", data.display()),
        &format!("--port={port}"),
        "--bind-address=127.0.0.1",
        "--console",
    ])
    .stdout(log.try_clone()?)
    .stderr(log)
    .stdin(Stdio::null())
    .spawn()?;
    let _server = Server(child);
    let client = bin.join(engine_kind.client());
    let start = Instant::now();
    loop {
        if sql(&client, port, "SELECT 1").is_ok() {
            break;
        }
        ensure!(
            start.elapsed() < Duration::from_secs(45),
            "Server startup timeout: {}",
            fs::read_to_string(temp.path().join("server.log"))?
        );
        std::thread::sleep(Duration::from_millis(300));
    }
    sql(
        &client,
        port,
        "CREATE DATABASE straja_source; CREATE DATABASE straja_restore; CREATE TABLE straja_source.orders(id INT PRIMARY KEY,note VARCHAR(100)); INSERT INTO straja_source.orders VALUES (1,'first'),(2,'second'); CREATE USER 'backup_reader'@'127.0.0.1' IDENTIFIED BY 'isolated-test-password'; GRANT SELECT,SHOW VIEW,TRIGGER,EVENT ON straja_source.* TO 'backup_reader'@'127.0.0.1';",
    )?;
    if engine_kind == Engine::MySql {
        sql(
            &client,
            port,
            "GRANT SHOW_ROUTINE ON *.* TO 'backup_reader'@'127.0.0.1'",
        )?;
    } else {
        sql(
            &client,
            port,
            "GRANT SELECT ON mysql.proc TO 'backup_reader'@'127.0.0.1'",
        )?;
    }
    let c = Connection {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Isolated test".into(),
        engine: engine_kind,
        host: "127.0.0.1".into(),
        port,
        user: "backup_reader".into(),
        tools: bin,
        tls: false,
        ca: String::new(),
    };
    let p = Plan {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Test".into(),
        connection: c.id.clone(),
        databases: vec!["straja_source".into()],
        destination: temp.path().into(),
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
        engine::databases(&c, "isolated-test-password", &temp.path().join("query"))?
            .contains(&"straja_source".into())
    );
    let before = sql(
        &client,
        port,
        "SELECT GROUP_CONCAT(CONCAT(id,note) ORDER BY id) FROM straja_source.orders",
    )?;
    let f = engine::backup(
        &p,
        &c,
        "isolated-test-password",
        &uuid::Uuid::new_v4().to_string(),
    )?;
    let m = engine::verify(&f)?;
    let mut raw = String::new();
    use std::io::Read;
    flate2::read::GzDecoder::new(fs::File::open(f.join(&m.files[0].file))?)
        .read_to_string(&mut raw)?;
    // Restore the generated SQL only to the isolated test database, never to the source.
    let restore = raw.replace("`straja_source`", "`straja_restore`");
    let file = temp.path().join("restore.sql");
    fs::write(&file, restore)?;
    let out = Command::new(&client)
        .args([
            "--no-defaults",
            "--protocol=TCP",
            "--host=127.0.0.1",
            &format!("--port={port}"),
            "--user=root",
        ])
        .stdin(fs::File::open(file)?)
        .output()?;
    ensure!(
        out.status.success(),
        "Restore: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        before,
        sql(
            &client,
            port,
            "SELECT GROUP_CONCAT(CONCAT(id,note) ORDER BY id) FROM straja_restore.orders"
        )?
    );
    assert_eq!(
        before,
        sql(
            &client,
            port,
            "SELECT GROUP_CONCAT(CONCAT(id,note) ORDER BY id) FROM straja_source.orders"
        )?
    );
    Ok(())
}
#[test]
#[ignore = "Requires local MySQL binaries; creates isolated temporary server"]
fn mysql_real_roundtrip() -> Result<()> {
    roundtrip(
        Engine::MySql,
        PathBuf::from(
            std::env::var("STRAJA_TEST_MYSQL_BIN")
                .unwrap_or("C:/laragon/bin/mysql/mysql-8.4.3-winx64/bin".into()),
        ),
    )
}
#[test]
#[ignore = "Requires local MariaDB binaries; creates isolated temporary server"]
fn mariadb_real_roundtrip() -> Result<()> {
    roundtrip(
        Engine::MariaDb,
        PathBuf::from(
            std::env::var("STRAJA_TEST_MARIA_BIN")
                .unwrap_or("C:/laragon/bin/mysql/mariadb-10.5.5-winx64/bin".into()),
        ),
    )
}
