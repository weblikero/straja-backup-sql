#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)] // Windows product executable name.
mod ui;
fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--agent") {
        return straja_sql::worker::launch();
    }
    let root = straja_sql::store::data_dir()?;
    std::fs::create_dir_all(&root)?;
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
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
                }
            }
            store.request_window(std::env::args().any(|a| a == "--activity"))?;
            return Ok(());
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    let _mutex = straja_sql::security::AppMutex::new("Local\\StrajaBackupSQL.Window")?;
    straja_sql::store::Store::open(&root)?.put("ui", "pid", &std::process::id())?;
    ui::run()?;
    Ok(())
}
