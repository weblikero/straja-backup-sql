//! Synthetic local benchmark. Never connects to SQL or touches user profiles.
use std::{
    fs::File,
    io::{BufWriter, Write},
    time::Instant,
};
use straja_sql::{engine, model::BackupProgress, store::Store};
fn cpu_seconds() -> f64 {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let mut c = FILETIME::default();
    let mut e = FILETIME::default();
    let mut k = FILETIME::default();
    let mut u = FILETIME::default();
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    (ticks(k) + ticks(u)) as f64 / 10_000_000.0
}
fn main() -> anyhow::Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "archive".into());
    let root = tempfile::tempdir()?;
    if mode == "archive" {
        let path = root.path().join("synthetic.sql");
        let mut file = BufWriter::new(File::create(&path)?);
        let mut bytes = 0;
        let mut value = 17u64;
        while bytes < 64 * 1024 * 1024 {
            value = value.wrapping_mul(6364136223846793005).wrapping_add(1);
            let line = format!(
                "INSERT INTO sample VALUES ({},'customer-{}','order-{}',123.45);\n",
                value % 100000,
                value % 1000,
                value
            );
            file.write_all(line.as_bytes())?;
            bytes += line.len();
        }
        file.flush()?;
        drop(file);
        let start = Instant::now();
        let cpu_start = cpu_seconds();
        let archive = engine::archive(&path, true, "synthetic")?;
        println!(
            "{}",
            serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"mode":mode,"input_bytes":bytes,"archive_bytes":archive.bytes,"elapsed_ms":start.elapsed().as_millis(),"cpu_seconds":cpu_seconds()-cpu_start})
        );
    } else {
        let store = Store::open(root.path())?;
        let tx = store.db.unchecked_transaction()?;
        for i in 0..10_000 {
            store.db.execute("INSERT INTO runs(id,plan_id,plan_name,status,started,detail) VALUES (?1,'plan','Synthetic','Reușit',?1,'')", [format!("{i:08}")])?;
            if i >= 9800 {
                store.put(
                    "progress",
                    &format!("{i:08}"),
                    &BackupProgress {
                        phase: "Comprimare".into(),
                        database: "synthetic".into(),
                        database_index: 1,
                        database_count: 1,
                        bytes: 100,
                        total_bytes: Some(100),
                        updated: chrono::Utc::now(),
                    },
                )?;
            }
        }
        tx.commit()?;
        let start = Instant::now();
        let cpu_start = cpu_seconds();
        for _ in 0..200 {
            assert_eq!(store.runs()?.len(), 200);
        }
        println!(
            "{}",
            serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"mode":mode,"history_rows":10000,"reads":200,"elapsed_ms":start.elapsed().as_millis(),"cpu_seconds":cpu_seconds()-cpu_start})
        );
    }
    Ok(())
}
