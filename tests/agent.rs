use anyhow::{Result, ensure};
use chrono::{Duration, Utc};
use std::{
    process::{Child, Command},
    time::Instant,
};
use straja_sql::{model::*, store::Store};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    UI::{Shell::*, WindowsAndMessaging::*},
};
struct WindowSearch {
    pid: u32,
    hwnd: HWND,
}
unsafe extern "system" fn find_tray(hwnd: HWND, context: LPARAM) -> i32 {
    unsafe {
        let search = &mut *(context as *mut WindowSearch);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == search.pid {
            let mut name = [0u16; 128];
            let len = GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32);
            if len > 0
                && String::from_utf16_lossy(&name[..len as usize]) == straja_sql::tray::WINDOW_CLASS
            {
                search.hwnd = hwnd;
                return 0;
            }
        }
        1
    }
}
struct Agent(Child);
impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn agent_recovers_durable_queue_rejects_duplicate_and_records_email_failure() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("StrajaSQL");
    let s = Store::open(&root)?;
    let schedule = Schedule {
        frequency: Frequency::Once,
        date: (Utc::now() - Duration::days(1)).date_naive(),
        ..Schedule::default()
    };
    let p = Plan {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Agent isolated test".into(),
        connection: uuid::Uuid::new_v4().to_string(),
        databases: vec!["test".into()],
        destination: temp.path().into(),
        remote: None,
        schedule,
        compression: Compression::Gzip,
        keep_count: None,
        keep_days: None,
        enabled: true,
        next_due: Some(Utc::now() - Duration::minutes(1)),
        notify: true,
        timeout_minutes: 1,
    };
    s.put("plan", &p.id, &p)?;
    // Missing connection deliberately fails before any networking. No SMTP configured.
    let mut agent = Agent(
        Command::new(env!("CARGO_BIN_EXE_StrajaBackupSQL-Agent"))
            .env("LOCALAPPDATA", temp.path())
            .spawn()?,
    );
    let start = Instant::now();
    loop {
        let runs = s.runs()?;
        if runs
            .iter()
            .any(|r| r.status == "Eroare" && r.detail.contains("Alert email was NOT sent"))
        {
            break;
        }
        ensure!(
            start.elapsed().as_secs() < 20,
            "Agent did not finish isolated failure"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(s.runs()?.len(), 1);
    assert!(s.get::<String>("agent", "tray_error")?.is_empty());
    let mut search = WindowSearch {
        pid: agent.0.id(),
        hwnd: std::ptr::null_mut(),
    };
    unsafe {
        EnumWindows(
            Some(find_tray),
            (&mut search as *mut WindowSearch) as LPARAM,
        );
    }
    ensure!(
        !search.hwnd.is_null(),
        "Agent did not create its notification window"
    );
    let identifier = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: search.hwnd,
        uID: 1,
        ..Default::default()
    };
    let mut rectangle = RECT::default();
    let check_start = Instant::now();
    let registered = loop {
        let result = unsafe { Shell_NotifyIconGetRect(&identifier, &mut rectangle) };
        if result >= 0 || check_start.elapsed().as_secs() >= 5 {
            break result;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    ensure!(
        registered >= 0,
        "Shell icon rectangle unavailable: {registered:#x}; NIM_ADD accepted={:?}",
        s.get::<bool>("agent", "tray_registered")
    );

    // The running agent adopts the saved language without a restart. Each job fails
    // at the missing connection, so there is no SQL or SMTP traffic.
    for (language, notification) in [
        ("de", "Warn-E-Mail wurde NICHT gesendet"),
        ("es", "NO se envió el correo de alerta"),
        ("pt", "O email de alerta NÃO foi enviado"),
        ("fr", "L’email d’alerte n’a PAS été envoyé"),
        ("it", "L’email di avviso NON è stata inviata"),
        ("en", "Alert email was NOT sent"),
    ] {
        s.put("settings", "language", &language)?;
        let previous: std::collections::HashSet<_> = s.runs()?.into_iter().map(|r| r.id).collect();
        s.enqueue(&p)?;
        let run_id = s
            .runs()?
            .into_iter()
            .find(|r| !previous.contains(&r.id))
            .unwrap()
            .id;
        let started = Instant::now();
        loop {
            if let Some(run) = s.runs()?.into_iter().find(|r| r.id == run_id)
                && run.status == "Eroare"
                && run.detail.contains(notification)
            {
                assert_eq!(run.plan_name, p.name);
                break;
            }
            ensure!(
                started.elapsed().as_secs() < 15,
                "Agent did not adopt language {language}"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    // An existing GUI receives requests through SQLite; no second GUI window opens.
    let window_lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("window.lock"))?;
    window_lock.try_lock()?;
    let status = Command::new(env!("CARGO_BIN_EXE_StrajaBackupSQL"))
        .arg("--activity")
        .env("LOCALAPPDATA", temp.path())
        .status()?;
    ensure!(status.success());
    assert_eq!(s.take_window_request()?, Some(true));
    let duplicate = Command::new(env!("CARGO_BIN_EXE_StrajaBackupSQL-Agent"))
        .env("LOCALAPPDATA", temp.path())
        .status()?;
    assert!(duplicate.success());
    let launcher = Command::new(env!("CARGO_BIN_EXE_StrajaBackupSQL"))
        .arg("--agent")
        .env("LOCALAPPDATA", temp.path())
        .status()?;
    assert!(launcher.success());
    assert!(agent.0.try_wait()?.is_none());
    straja_sql::tray::request_stop(&root)?;
    let start = Instant::now();
    loop {
        if let Some(status) = agent.0.try_wait()? {
            assert!(status.success());
            break;
        }
        ensure!(start.elapsed().as_secs() < 10, "Agent did not stop");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(s.snapshot()?.heartbeat.is_none());
    assert_eq!(unsafe { IsWindow(search.hwnd) }, 0);
    assert!(unsafe { Shell_NotifyIconGetRect(&identifier, &mut rectangle) } < 0);
    Ok(())
}
