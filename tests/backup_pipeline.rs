//! Real child-process/file pipeline with a deterministic client fixture; no SQL/network.
use anyhow::{Result, ensure};
use chrono::Utc;
use std::{
    fs,
    io::{Write, stdout},
    time::Duration,
};
use straja_sql::{engine, model::*, store::Store};

fn main() -> Result<()> {
    let exe = std::env::current_exe()?;
    let stem = exe.file_stem().unwrap().to_string_lossy();
    if stem == "mysql" || stem == "mysqldump" || stem == "psql" {
        use windows_sys::Win32::System::Threading::{
            BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, GetPriorityClass,
        };
        ensure!(
            unsafe { GetPriorityClass(GetCurrentProcess()) } == BELOW_NORMAL_PRIORITY_CLASS,
            "SQL client fixture must run below normal CPU priority"
        );
        let args = std::env::args().collect::<Vec<_>>();
        if args.iter().any(|s| s == "--host=fail") {
            eprintln!("fixture failure");
            std::process::exit(1);
        }
        if stem != "mysqldump" {
            println!("7075626c6963\t636f6d656e7a69\tr");
            println!("7075626c6963\t766564657265\tv");
        } else {
            let mut out = stdout().lock();
            for _ in 0..4 {
                out.write_all(&vec![b'x'; 262144])?;
                out.flush()?;
                std::thread::sleep(Duration::from_millis(220));
            }
            out.write_all(b"\n-- Dump completed on fixture\n")?;
        }
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    let bin = temp.path().join("clients");
    fs::create_dir(&bin)?;
    for name in ["mysql.exe", "mysqldump.exe", "psql.exe", "pg_dump.exe"] {
        fs::copy(&exe, bin.join(name))?;
    }
    let mut c = Connection {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Fixture".into(),
        engine: Engine::MySql,
        host: "fixture.invalid".into(),
        port: 3306,
        user: "backup".into(),
        tools: bin.clone(),
        tls: true,
        ca: String::new(),
    };
    let mut p = Plan {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Fixture".into(),
        connection: c.id.clone(),
        databases: vec!["magazin".into(), "Facturi 2026".into()],
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
    let store = Store::open(&temp.path().join("state"))?;
    store.enqueue(&p)?;
    let (run_id, _) = store.claim()?.unwrap();
    let mut events = vec![];
    let folder = engine::backup_with_progress(&p, &c, "never-visible", &run_id, &mut |event| {
        store.put("progress", &run_id, &event).unwrap();
        events.push(event);
    })?;
    let manifest = engine::verify(&folder)?;
    ensure!(manifest.version == 2);
    ensure!(manifest.files[0].file == "magazin.sql.gz");
    ensure!(manifest.files[1].file == "Facturi 2026.sql.gz");
    ensure!(
        fs::read_dir(&folder)?.count() == 3,
        "No raw SQL or stderr should remain"
    );
    ensure!(
        events
            .iter()
            .any(|e| e.phase == "Export" && e.bytes > 0 && e.percent().is_none())
    );
    for phase in [
        "Comprimare",
        "Verificare export",
        "Verificare GZIP",
        "Verificare fișier",
        "Validare finală · conținut",
    ] {
        ensure!(
            events
                .iter()
                .any(|e| e.phase == phase && e.percent() == Some(100.)),
            "Missing progress: {phase}"
        );
    }
    let reopened = Store::open(&temp.path().join("state"))?;
    ensure!(reopened.runs()?[0].progress.as_ref().unwrap().database == "Facturi 2026");
    store.finish(&run_id, "Reușit", "fixture", Some(&folder))?;
    ensure!(store.runs()?[0].status == "Reușit");
    // The same reader path works for both client families, including metadata parsing.
    for engine in [Engine::MySql, Engine::PostgreSql] {
        c.engine = engine;
        let tables = engine::tables(&c, "", "db'quote", &temp.path().join("query"))?;
        ensure!(tables.len() == 2 && tables[0].name == "comenzi" && tables[1].kind == "Vedere");
    }
    c.engine = Engine::MySql;
    p.compression = Compression::None;
    p.databases = vec!["shop".into()];
    let next = engine::backup(&p, &c, "", &uuid::Uuid::new_v4().to_string())?;
    ensure!(next.join("shop.sql").is_file());
    ensure!(engine::retention(&p, &next, Utc::now())? == 1);
    ensure!(!folder.exists() && next.exists());
    c.host = "fail".into();
    ensure!(engine::backup(&p, &c, "", &uuid::Uuid::new_v4().to_string()).is_err());
    ensure!(engine::verify(&next).is_ok());
    // A stop request from the tray waits for the running export and verification.
    let profile = temp.path().join("agent-profile");
    let root = profile.join("StrajaSQL");
    let mut agent_store = Store::open(&root)?;
    c.host = "fixture.invalid".into();
    agent_store.save_connection(&c, "fixture")?;
    p.keep_count = None;
    agent_store.save_plan(&p)?;
    agent_store.enqueue(&p)?;
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut agent = ChildGuard(
        std::process::Command::new(env!("CARGO_BIN_EXE_StrajaBackupSQL-Agent"))
            .env("LOCALAPPDATA", &profile)
            .spawn()?,
    );
    let started = std::time::Instant::now();
    loop {
        if agent_store
            .runs()?
            .first()
            .is_some_and(|r| r.progress.as_ref().is_some_and(|p| p.phase == "Export"))
        {
            break;
        }
        ensure!(
            started.elapsed().as_secs() < 15,
            "Agent did not start the fixture export"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    straja_sql::tray::request_stop(&root)?;
    ensure!(
        agent.0.try_wait()?.is_none(),
        "Tray stop interrupted the export"
    );
    loop {
        if let Some(status) = agent.0.try_wait()? {
            ensure!(status.success());
            break;
        }
        ensure!(
            started.elapsed().as_secs() < 20,
            "Agent did not stop after export"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let runs = agent_store.runs()?;
    ensure!(runs[0].status == "Reușit");
    ensure!(engine::verify(runs[0].folder.as_ref().unwrap()).is_ok());
    println!(
        "backup_pipeline: progress, persisted state, metadata, named files, retention and failure preservation passed; no SQL servers used."
    );
    Ok(())
}
