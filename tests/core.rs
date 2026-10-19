use chrono::{DateTime, Duration, Utc};
use std::{fs, path::Path};
use straja_sql::{
    engine::{self, Manifest},
    model::*,
    security,
    store::Store,
};
fn dt(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
fn plan(path: &Path) -> Plan {
    Plan {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Test".into(),
        connection: uuid::Uuid::new_v4().to_string(),
        databases: vec!["shop".into()],
        destination: path.into(),
        schedule: Schedule::default(),
        compression: Compression::Gzip,
        keep_count: None,
        keep_days: None,
        enabled: true,
        next_due: None,
        notify: false,
        timeout_minutes: 1,
    }
}
fn fixture(p: &Plan, age: i64) -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4().to_string();
    let folder = p
        .destination
        .join(format!("straja-plan-{}", p.id))
        .join(format!("backup-{id}"));
    fs::create_dir_all(&folder).unwrap();
    let raw = folder.join("database-001.sql");
    fs::write(
        &raw,
        "CREATE TABLE test(id INT);\n-- Dump completed on test\n",
    )
    .unwrap();
    let file = engine::archive(&raw, true, "shop").unwrap();
    let manifest = Manifest {
        version: 1,
        plan_id: p.id.clone(),
        run_id: id,
        created: Utc::now() - Duration::days(age),
        engine: Engine::MySql,
        files: vec![file],
    };
    fs::write(
        folder.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    folder
}
#[test]
fn dst_month_end_and_once() {
    let mut s = Schedule {
        hour: 3,
        minute: 30,
        timezone: "Europe/Bucharest".into(),
        ..Schedule::default()
    };
    assert_eq!(
        s.next_after(dt("2026-03-28T23:00:00Z")),
        Some(dt("2026-03-30T00:30:00Z"))
    );
    assert_eq!(
        s.next_after(dt("2026-10-25T00:31:00Z")),
        Some(dt("2026-10-26T01:30:00Z"))
    );
    s.frequency = Frequency::Monthly;
    s.day = 31;
    s.hour = 2;
    s.minute = 0;
    assert_eq!(
        s.next_after(dt("2026-02-01T00:00:00Z")),
        Some(dt("2026-02-28T00:00:00Z"))
    );
    s.frequency = Frequency::Once;
    s.date = "2026-02-01".parse().unwrap();
    assert_eq!(s.next_after(dt("2026-02-02T00:00:00Z")), None);
    s.date = "2035-01-01".parse().unwrap();
    assert_eq!(
        s.next_after(dt("2026-02-02T00:00:00Z")),
        Some(dt("2035-01-01T00:00:00Z"))
    );
}
#[test]
fn schedule_validation_and_intervals() {
    let mut s = Schedule {
        frequency: Frequency::Minutes,
        minutes: 5,
        ..Schedule::default()
    };
    let now = dt("2026-01-01T00:00:00Z");
    assert_eq!(s.next_after(now), Some(now + Duration::minutes(5)));
    s.hour = 24;
    assert!(s.validate().is_err());
    assert_eq!(s.next_after(now), None);
    s.hour = 1;
    s.timezone = "Invalid/Zone".into();
    assert!(s.validate().is_err());
}
#[test]
fn malicious_database_names_rejected() {
    for db in [
        "--result-file=C:/evil",
        "postgresql://evil/db",
        "host=evil",
        "test\nDROP TABLE x",
        "",
    ] {
        assert!(validate_database(db).is_err());
    }
    assert!(validate_database("magazin-2026").is_ok());
}
#[test]
fn archive_corruption_detected() {
    let t = tempfile::tempdir().unwrap();
    let p = plan(t.path());
    let folder = fixture(&p, 0);
    assert!(engine::verify(&folder).is_ok());
    fs::write(folder.join("database-001.sql.gz"), b"corrupt").unwrap();
    assert!(engine::verify(&folder).is_err());
}
#[test]
fn manifest_traversal_rejected() {
    let t = tempfile::tempdir().unwrap();
    let p = plan(t.path());
    let f = fixture(&p, 0);
    let mut m: Manifest =
        serde_json::from_slice(&fs::read(f.join("manifest.json")).unwrap()).unwrap();
    m.files[0].file = "../outside.sql".into();
    fs::write(f.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(engine::verify(&f).is_err());
}
#[test]
fn retention_preserves_latest_foreign_and_partial() {
    let t = tempfile::tempdir().unwrap();
    let mut p = plan(t.path());
    p.keep_count = Some(1);
    p.keep_days = Some(1);
    let old = fixture(&p, 30);
    let foreign = fixture(&p, 20);
    fs::write(foreign.join("personal.txt"), "keep").unwrap();
    let new = fixture(&p, 0);
    let partial = new.parent().unwrap().join("partial-abandoned");
    fs::create_dir(&partial).unwrap();
    assert_eq!(engine::retention(&p, &new, Utc::now()).unwrap(), 1);
    assert!(!old.exists());
    assert!(new.exists());
    assert!(foreign.join("personal.txt").exists());
    assert!(partial.exists());
}
#[test]
fn retention_never_deletes_other_plan() {
    let t = tempfile::tempdir().unwrap();
    let mut p = plan(t.path());
    p.keep_count = Some(1);
    let another = plan(t.path());
    let foreign = fixture(&another, 30);
    assert!(engine::retention(&p, &foreign, Utc::now()).is_err());
    assert!(foreign.exists());
}
#[test]
fn dpapi_encrypts_and_roundtrips() {
    let bytes = b"password-never-in-database";
    let encrypted = security::protect(bytes).unwrap();
    assert!(!encrypted.windows(bytes.len()).any(|w| w == bytes));
    assert_eq!(security::unprotect(&encrypted).unwrap(), bytes);
}
#[test]
fn durable_queue_recovery_and_no_duplicate() {
    let t = tempfile::tempdir().unwrap();
    let s = Store::open(t.path()).unwrap();
    let p = plan(t.path());
    s.put("plan", &p.id, &p).unwrap();
    s.enqueue(&p).unwrap();
    assert!(s.enqueue(&p).is_err());
    assert!(s.delete("plan", &p.id).is_err());
    assert!(s.claim().unwrap().is_some());
    assert!(s.claim().unwrap().is_none());
    s.recover().unwrap();
    assert_eq!(s.runs().unwrap()[0].status, "Întrerupt");
    s.enqueue(&p).unwrap();
}
#[test]
fn missed_schedule_enqueues_only_once() {
    let t = tempfile::tempdir().unwrap();
    let s = Store::open(t.path()).unwrap();
    let mut p = plan(t.path());
    let now = Utc::now();
    p.next_due = Some(now - Duration::days(30));
    s.put("plan", &p.id, &p).unwrap();
    s.tick_schedule(now).unwrap();
    s.tick_schedule(now).unwrap();
    assert_eq!(s.runs().unwrap().len(), 1);
    let updated: Plan = s.get("plan", &p.id).unwrap();
    assert!(updated.next_due.unwrap() > now);
}
#[test]
fn command_does_not_expose_password_or_allow_mutation() {
    for engine in Engine::ALL {
        let c = Connection {
            id: uuid::Uuid::new_v4().to_string(),
            name: "test".into(),
            engine,
            host: "localhost".into(),
            port: engine.port(),
            user: "backup".into(),
            tools: "C:/tools".into(),
            tls: true,
            ca: String::new(),
        };
        let cmd = engine::connection_command(&c, "sensitive-password", true).unwrap();
        let args = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(!args.iter().any(|a| a.contains("sensitive-password")));
        assert!(
            !args
                .iter()
                .any(|a| a.contains("delete") || a.contains("drop") || a.contains("flush"))
        );
    }
}

#[test]
fn bundled_tools_are_resolved_next_to_executable_and_custom_paths_are_preserved() {
    let root = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    for engine in Engine::ALL {
        let mut c = Connection {
            id: uuid::Uuid::new_v4().to_string(),
            name: "test".into(),
            engine,
            host: "localhost".into(),
            port: engine.port(),
            user: "backup".into(),
            tools: Default::default(),
            tls: true,
            ca: String::new(),
        };
        let expected = root.join("tools").join(engine.tools_folder()).join("bin");
        for dump in [false, true] {
            let cmd = engine::connection_command(&c, "", dump).unwrap();
            assert_eq!(
                cmd.get_program(),
                expected.join(if dump { engine.dump() } else { engine.client() })
            );
            assert_eq!(cmd.get_current_dir(), Some(expected.as_path()));
        }
        // Missing bundled files fail validation; there is no search through PATH.
        assert!(c.validate().is_err());
        let custom = tempfile::tempdir().unwrap();
        c.tools = custom.path().into();
        fs::write(custom.path().join(engine.client()), b"test fixture").unwrap();
        fs::write(custom.path().join(engine.dump()), b"test fixture").unwrap();
        assert_eq!(c.tools_dir().unwrap(), custom.path());
        assert!(c.validate().is_ok());
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(
            serde_json::from_str::<Connection>(&json).unwrap().tools,
            c.tools
        );
        c.tools = "relative-folder".into();
        assert!(c.tools_dir().is_err());
        assert!(engine::connection_command(&c, "", false).is_err());
    }
}

#[test]
fn original_database_filenames_are_safe_and_case_collision_free() {
    let dbs = [
        "magazin",
        "Bază 2026",
        "Shop",
        "shop",
        "shop--4-1",
        "CON",
        "aux.txt",
        "../outside",
        "name:stream",
        "ends. ",
        "a/b",
        "a_b",
    ]
    .map(String::from);
    let names = engine::backup_filenames(&dbs);
    assert_eq!(&names[..3], &["magazin.sql", "Bază 2026.sql", "Shop.sql"]);
    let unique = names
        .iter()
        .map(|n| n.to_lowercase())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), names.len());
    let folder = tempfile::tempdir().unwrap();
    for name in &names {
        assert!(!name.chars().any(|c| "<>:\"/\\|?*".contains(c)));
        assert!(!name.starts_with('.'));
        let path = folder.path().join(name);
        fs::File::options()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        engine::ordinary(&path, false).unwrap();
    }
    assert_ne!(names[5], "CON.sql");
    assert_ne!(names[6], "aux.txt.sql");
    assert_eq!(names, engine::backup_filenames(&dbs));
    let long = engine::backup_filenames(&["ă".repeat(200)]);
    assert!(long[0].encode_utf16().count() < 255);
}

#[test]
fn named_manifest_verification_rejects_traversal_and_preserves_legacy() {
    let t = tempfile::tempdir().unwrap();
    let p = plan(t.path());
    let folder = fixture(&p, 0);
    let mut manifest = engine::verify(&folder).unwrap(); // Legacy v1 remains supported.
    fs::rename(
        folder.join("database-001.sql.gz"),
        folder.join("shop.sql.gz"),
    )
    .unwrap();
    manifest.version = 2;
    manifest.files[0].file = "shop.sql.gz".into();
    fs::write(
        folder.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(engine::verify(&folder).is_ok());
    manifest.files[0].file = "../shop.sql.gz".into();
    fs::write(
        folder.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(engine::verify(&folder).is_err());
}

#[test]
fn metadata_names_are_encoded_and_database_is_never_interpolated_into_sql() {
    let tables =
        engine::parse_tables("7075626c6963\t61270962\tBASE TABLE\n7075626c6963\tc89974\tv\n")
            .unwrap();
    assert_eq!(tables[0].name, "a'\tb");
    assert_eq!(tables[1].name, "șt");
    assert!(engine::parse_tables("garbage").is_err());
    assert!(engine::parse_tables("00\txyz\tr").is_err());
    assert!(engine::parse_tables("").unwrap().is_empty());
    for engine in Engine::ALL {
        let c = Connection {
            id: uuid::Uuid::new_v4().to_string(),
            name: "test".into(),
            engine,
            host: "fixture.invalid".into(),
            port: engine.port(),
            user: "backup".into(),
            tools: "C:/tools".into(),
            tls: true,
            ca: String::new(),
        };
        let database = "quote'; DROP TABLE x; --";
        let cmd = engine::tables_command(&c, "secret", database).unwrap();
        let sql = cmd
            .get_args()
            .find(|a| {
                a.to_string_lossy().starts_with("--command=")
                    || a.to_string_lossy().starts_with("--execute=")
            })
            .unwrap()
            .to_string_lossy();
        assert!(sql.contains("SELECT "));
        assert!(!sql.contains(database));
        assert!(!sql.contains("secret"));
    }
}

#[test]
fn progress_percentage_is_measured_and_unknown_export_has_no_percentage() {
    let mut progress = BackupProgress {
        phase: "Export".into(),
        database: "db".into(),
        database_index: 1,
        database_count: 1,
        bytes: 25,
        total_bytes: None,
        updated: Utc::now(),
    };
    assert_eq!(progress.percent(), None);
    progress.total_bytes = Some(100);
    assert_eq!(progress.percent(), Some(25.));
    progress.bytes = 101;
    assert_eq!(progress.percent(), Some(100.));
    progress.total_bytes = Some(0);
    assert_eq!(progress.percent(), None);
}
#[test]
fn lightweight_history_and_tray_keep_active_runs_and_tolerate_missing_progress()
-> anyhow::Result<()> {
    use straja_sql::store::Store;
    let temp = tempfile::tempdir()?;
    let store = Store::open(temp.path())?;
    let tx = store.db.unchecked_transaction()?;
    for i in 0..250 {
        store.db.execute("INSERT INTO runs(id,plan_id,plan_name,status,started,detail) VALUES (?1,?1,'test','Reușit',?1,'')", [format!("{i:04}")])?;
    }
    store
        .db
        .execute("UPDATE runs SET status='În lucru' WHERE id='0000'", [])?;
    store.db.execute(
        "INSERT INTO objects VALUES ('progress','0249','malformed')",
        [],
    )?;
    tx.commit()?;
    let history = store.runs()?;
    assert_eq!(history.len(), 200);
    assert_eq!(history[0].id, "0249");
    assert!(history[0].progress.is_none());
    assert!(history.iter().all(|r| r.id != "0000"));
    assert_eq!(store.tray_runs()?.len(), 1);
    assert_eq!(store.tray_runs()?[0].id, "0000");
    store.finish("0000", "Reușit", "done", None)?;
    assert_eq!(store.tray_runs()?[0].id, "0249");
    let query_plan: String = store.db.query_row(
        "EXPLAIN QUERY PLAN SELECT id FROM runs ORDER BY started DESC LIMIT 200",
        [],
        |r| r.get(3),
    )?;
    assert!(query_plan.contains("runs_by_started"));
    Ok(())
}
