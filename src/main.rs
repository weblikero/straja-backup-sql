#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)] // Windows product executable name.
mod ui;
fn main() -> anyhow::Result<()> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.iter().any(|a| a == "--version") {
        println!(
            "StrajaBackupSQL {} {}",
            env!("CARGO_PKG_VERSION"),
            straja_sql::platform::ID
        );
        return Ok(());
    }
    if arguments.iter().any(|a| a == "--self-check") {
        return straja_sql::platform::self_check();
    }
    #[cfg(windows)]
    if arguments.get(1).is_some_and(|a| a == "--apply-update") {
        anyhow::ensure!(arguments.len() == 3, "Invalid update handoff");
        let result = straja_sql::updater::apply(std::path::Path::new(&arguments[2]));
        if let Err(e) = &result {
            straja_sql::updater::notify_error(e);
        }
        return result;
    }
    if std::env::args().any(|a| a == "--agent") {
        return straja_sql::worker::launch();
    }
    let root = straja_sql::store::data_dir()?;
    std::fs::create_dir_all(&root)?;
    let coordination = match straja_sql::updater::startup_coordination(&root) {
        Ok(lock) => lock,
        Err(_) => return Ok(()),
    };
    let window_lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("window.lock"))?;
    match window_lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            let store = straja_sql::store::Store::open(&root)?;
            if let Ok(pid) = store.get::<u32>("ui", "pid") {
                #[cfg(windows)]
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
                }
                #[cfg(unix)]
                let _ = pid;
            }
            store.request_window(std::env::args().any(|a| a == "--activity"))?;
            return Ok(());
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    let _mutex = straja_sql::security::AppMutex::new("Local\\StrajaBackupSQL.Window")?;
    drop(coordination);
    straja_sql::store::Store::open(&root)?.put("ui", "pid", &std::process::id())?;
    #[cfg(unix)]
    let _ = straja_sql::worker::launch();
    ui::run()?;
    Ok(())
}
