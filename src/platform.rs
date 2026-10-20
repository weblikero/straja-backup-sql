use anyhow::{Context, Result};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(windows)]
pub const ID: &str = "windows-x64";
#[cfg(target_os = "linux")]
pub const ID: &str = "linux-x64";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const ID: &str = "macos-arm64";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const ID: &str = "macos-x64";

pub fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}
pub fn artifact(version: &str) -> String {
    if cfg!(windows) {
        format!("StrajaBackupSQL-{version}-{ID}-setup.exe")
    } else if cfg!(target_os = "macos") {
        format!("StrajaBackupSQL-{version}-{ID}.pkg")
    } else {
        format!("StrajaBackupSQL-{version}-{ID}.deb")
    }
}
pub fn data_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(
            PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is missing")?)
                .join("StrajaSQL"),
        )
    }
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or(
                PathBuf::from(std::env::var_os("HOME").context("HOME is missing")?)
                    .join(".local/share"),
            );
        Ok(base.join("StrajaBackupSQL"))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(
            PathBuf::from(std::env::var_os("HOME").context("HOME is missing")?)
                .join("Library/Application Support/StrajaBackupSQL"),
        )
    }
}
pub fn tools_root() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(exe
        .parent()
        .context("Application directory missing")?
        .join("tools"))
}
pub fn restricted_directory(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
            "Profile must not be a symlink"
        );
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn open(path: &Path) -> Result<()> {
    let program = if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else {
        "/usr/bin/xdg-open"
    };
    Command::new(program).arg(path).spawn()?;
    Ok(())
}
pub fn open_installer(path: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        Command::new("/usr/bin/gdebi-gtk").arg(path).spawn()?;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        open(path)
    }
}
pub fn low_priority(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(
            0x08000000 | windows_sys::Win32::System::Threading::BELOW_NORMAL_PRIORITY_CLASS,
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Only the child is affected. nice is async-signal-safe and cannot raise priority here.
        unsafe {
            command.pre_exec(|| {
                libc::nice(5);
                Ok(())
            });
        }
    }
}
pub fn self_check() -> Result<()> {
    println!("StrajaBackupSQL {} {}", env!("CARGO_PKG_VERSION"), ID);
    for engine in crate::model::Engine::ALL {
        for tool in [engine.client(), engine.dump()] {
            let path = tools_root()?
                .join(engine.tools_folder())
                .join("bin")
                .join(tool);
            let output = Command::new(&path)
                .arg("--version")
                .output()
                .with_context(|| format!("Could not run {}", path.display()))?;
            anyhow::ensure!(
                output.status.success(),
                "{}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr)
            );
            println!("{}", String::from_utf8_lossy(&output.stdout).trim());
        }
    }
    let output = Command::new(tools_root()?.join("transfer").join(executable("rclone")))
        .arg("version")
        .output()?;
    anyhow::ensure!(output.status.success(), "Transfer tool failed");
    println!("{}", String::from_utf8_lossy(&output.stdout).trim());
    Ok(())
}
