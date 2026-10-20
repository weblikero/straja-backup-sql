//! The initial Unix packages expose agent controls in Settings rather than a tray.
use anyhow::Result;
use std::path::Path;
pub struct Tray;
impl Tray {
    pub fn start(_: std::path::PathBuf) -> Result<Self> {
        Ok(Self)
    }
}
pub fn request_stop(root: &Path) -> Result<()> {
    crate::store::Store::open(root)?.put("agent", "stop", &true)
}
pub fn open_window(activity: bool) -> Result<()> {
    let path =
        std::env::current_exe()?.with_file_name(crate::platform::executable("StrajaBackupSQL"));
    let mut command = crate::engine::command(path);
    if activity {
        command.arg("--activity");
    }
    command.spawn()?;
    Ok(())
}
