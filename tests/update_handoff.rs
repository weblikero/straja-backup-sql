//! Exercises the real updater with a fake installer/agent/UI in a private LOCALAPPDATA.
//! Never executes Inno Setup, connects to SQL, or uses the user's installed application.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    fs::File,
    path::Path,
    process::Child,
    time::{Duration, Instant},
};
use straja_sql::{
    engine,
    store::{self, Store},
    updater,
};

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait(mut predicate: impl FnMut() -> bool) -> Result<()> {
    let start = Instant::now();
    while !predicate() {
        ensure!(
            start.elapsed() < Duration::from_secs(20),
            "Fixture timed out"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
fn lock(root: &Path, name: &str) -> Result<File> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(name))?;
    file.try_lock()?;
    Ok(file)
}
fn main() -> Result<()> {
    let exe = std::env::current_exe()?;
    let name = exe.file_name().unwrap().to_string_lossy();
    if name == "StrajaBackupSQL-Updater.exe" {
        let path = std::env::args_os().nth(2).unwrap();
        return updater::apply(Path::new(&path));
    }
    if name.starts_with("StrajaBackupSQL-99.") {
        let root = store::data_dir()?;
        ensure!(
            root.join("backup-finished").exists(),
            "Installer started during backup"
        );
        ensure!(
            root.join("ui-closed").exists(),
            "Installer started before UI closed"
        );
        fs::write(
            root.join("installer-args.json"),
            serde_json::to_vec(&std::env::args().collect::<Vec<_>>())?,
        )?;
        if std::env::var("FIXTURE_FAIL_INSTALLER").is_ok() {
            std::process::exit(5);
        }
        return Ok(());
    }
    if name == "StrajaBackupSQL.exe" {
        fs::write(store::data_dir()?.join("ui-restarted"), b"yes")?;
        return Ok(());
    }
    if name == "StrajaBackupSQL-Agent.exe" {
        let root = store::data_dir()?;
        if std::env::var("FIXTURE_BUSY_BACKUP").is_ok() {
            let _lock = lock(&root, "agent.lock")?;
            let store = Store::open(&root)?;
            store.put("agent", "executable", &exe)?;
            store.put("agent", "stop", &false)?;
            fs::write(root.join("agent-ready"), b"yes")?;
            wait(|| root.join("allow-backup-finish").is_file())?;
            ensure!(
                store.get::<bool>("agent", "stop")?,
                "Agent not asked to drain"
            );
            fs::write(root.join("backup-finished"), b"complete")?;
        } else {
            fs::write(root.join("agent-restarted"), b"yes")?;
        }
        return Ok(());
    }
    for failed in [false, true] {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("StrajaSQL");
        let target = temp.path().join("Installed app");
        let folder = root.join("updates").join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&folder)?;
        fs::create_dir(&target)?;
        fs::copy(&exe, target.join("StrajaBackupSQL.exe"))?;
        fs::copy(&exe, target.join("StrajaBackupSQL-Agent.exe"))?;
        fs::write(target.join("unins000.exe"), b"installer marker")?;
        fs::write(root.join("backup.sql"), b"preserve backup")?;
        let store = Store::open(&root)?;
        store.put("test", "settings", &"preserve settings")?;
        let helper = folder.join("StrajaBackupSQL-Updater.exe");
        fs::copy(&exe, &helper)?;
        let installer = folder.join("StrajaBackupSQL-99.0.0-windows-x64-setup.exe");
        fs::copy(&exe, &installer)?;
        let hash = format!("{:x}", Sha256::digest(fs::read(&exe)?));
        let job = serde_json::json!({
            "prepared":{"version":"99.0.0", "path":installer, "bytes":fs::metadata(&exe)?.len(), "sha256":hash},
            "target":target, "app_hash":hash, "language":"en"
        });
        fs::write(folder.join("job.json"), serde_json::to_vec(&job)?)?;
        let window = lock(&root, "window.lock")?;
        let mut agent = ChildGuard(
            engine::command(target.join("StrajaBackupSQL-Agent.exe"))
                .env("LOCALAPPDATA", temp.path())
                .env("FIXTURE_BUSY_BACKUP", "1")
                .spawn()?,
        );
        wait(|| root.join("agent-ready").exists())?;
        let mut command = engine::command(helper);
        command
            .env("LOCALAPPDATA", temp.path())
            .arg("--apply-update")
            .arg(folder.join("job.json"));
        if failed {
            command.env("FIXTURE_FAIL_INSTALLER", "1");
        }
        let mut helper = ChildGuard(command.spawn()?);
        wait(|| store.get::<bool>("agent", "stop").unwrap_or(false))?;
        ensure!(
            updater::coordination(&root).is_err(),
            "New claims must be blocked"
        );
        ensure!(
            !folder.join("ready").exists() && !root.join("installer-args.json").exists(),
            "Backup interrupted"
        );
        fs::write(root.join("allow-backup-finish"), b"go")?;
        wait(|| folder.join("ready").exists())?;
        ensure!(agent.0.wait()?.success(), "Fixture backup failed");
        ensure!(
            !root.join("installer-args.json").exists(),
            "UI still holds its lock"
        );
        fs::write(folder.join("commit"), b"install")?;
        fs::write(root.join("ui-closed"), b"yes")?;
        drop(window);
        let mut exit = None;
        wait(|| {
            exit = helper.0.try_wait().ok().flatten();
            exit.is_some()
        })?;
        ensure!(
            exit.unwrap().success() != failed,
            "Unexpected installer result"
        );
        wait(|| root.join("agent-restarted").exists() && root.join("ui-restarted").exists())?;
        let args: Vec<String> =
            serde_json::from_slice(&fs::read(root.join("installer-args.json"))?)?;
        for expected in [
            "/SILENT",
            "/NORESTART",
            "/NOCLOSEAPPLICATIONS",
            "/NORESTARTAPPLICATIONS",
            "/LANG=en",
        ] {
            ensure!(args.iter().any(|a| a == expected), "Missing {expected}");
        }
        ensure!(
            args.contains(&format!("/DIR={}", target.display())),
            "Wrong installation target"
        );
        ensure!(
            fs::read(root.join("backup.sql"))? == b"preserve backup",
            "Backup changed"
        );
        ensure!(
            store.get::<String>("test", "settings")? == "preserve settings",
            "Settings changed"
        );
        ensure!(installer.exists() == failed, "Installer cleanup mismatch");
        ensure!(
            folder.join("error.txt").exists() == failed,
            "Failure not recorded"
        );
        ensure!(updater::coordination(&root).is_ok(), "Update lock leaked");
        println!("Handoff fixture passed: installer failure={failed}");
    }
    Ok(())
}
