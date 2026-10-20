//! Download in the UI; hand off to a copy of our executable outside the installation.
//! Only this product's agent is drained. Never force-terminate applications or backups.
use crate::{
    engine,
    i18n::tr,
    store::{self, Store},
    updates,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

pub const MAX_INSTALLER: u64 = 512 * 1024 * 1024;
const REPO: &str = "weblikero/straja-backup-sql";
#[derive(Debug, Default)]
pub struct Progress {
    pub bytes: AtomicU64,
    pub total: AtomicU64,
    // 0 metadata, 1 download, 2 integrity check, 3 wait for agent/handoff.
    pub stage: AtomicUsize,
    pub cancel: AtomicBool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prepared {
    version: String,
    path: PathBuf,
    bytes: u64,
    sha256: String,
}
fn numeric_version(version: &str) -> Result<()> {
    let parsed = semver::Version::parse(version)?;
    ensure!(
        parsed.pre.is_empty() && parsed.build.is_empty() && parsed.to_string() == version,
        "Invalid installer version"
    );
    ensure!(
        parsed > semver::Version::parse(updates::CURRENT_VERSION)?,
        "Update is not newer"
    );
    Ok(())
}
fn installer_name(version: &str) -> String {
    format!("StrajaBackupSQL-{version}-windows-x64-setup.exe")
}
fn asset_metadata(bytes: &[u8], requested: &str) -> Result<(u64, String, String)> {
    numeric_version(requested)?;
    let updates::CheckResult::Available(release) =
        updates::parse_github(bytes, updates::CURRENT_VERSION)?
    else {
        bail!("Update is not available")
    };
    ensure!(release.version == requested, "Release version changed");
    let github: updates::GithubRelease = serde_json::from_slice(bytes)?;
    let name = installer_name(requested);
    let matching: Vec<_> = github.assets.iter().filter(|a| a.name == name).collect();
    ensure!(matching.len() == 1, "Ambiguous installer asset");
    let asset = matching[0];
    let url = format!("https://github.com/{REPO}/releases/download/v{requested}/{name}");
    ensure!(
        asset.browser_download_url == url,
        "Installer belongs to another product"
    );
    ensure!(
        asset.size > 0 && asset.size <= MAX_INSTALLER,
        "Installer size exceeds limit"
    );
    let digest = asset
        .digest
        .as_deref()
        .and_then(|s| s.strip_prefix("sha256:"))
        .context("GitHub has not provided an installer SHA-256")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid SHA-256"
    );
    Ok((asset.size, digest.to_ascii_lowercase(), url))
}
fn allowed_download(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
fn client(download: bool) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .https_only(true)
        .user_agent(concat!("StrajaBackupSQL/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(if download { 3600 } else { 30 }))
        .redirect(if download {
            reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 5 && allowed_download(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("Untrusted update redirect")
                }
            })
        } else {
            reqwest::redirect::Policy::none()
        })
        .build()?)
}
fn plain(path: &Path) -> Result<()> {
    ensure!(
        std::fs::symlink_metadata(path)?.file_attributes() & 0x400 == 0,
        "Update path cannot be a reparse point"
    );
    Ok(())
}
fn cache(root: &Path) -> Result<PathBuf> {
    plain(root)?;
    let path = root.join("updates");
    std::fs::create_dir_all(&path)?;
    plain(&path)?;
    Ok(path)
}
struct Partial(PathBuf);
impl Drop for Partial {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
async fn receive(
    mut response: reqwest::Response,
    path: &Path,
    size: u64,
    expected: &str,
    progress: &Progress,
) -> Result<()> {
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "Installer server returned {}",
        response.status()
    );
    ensure!(
        response.content_length().is_none_or(|n| n == size),
        "Installer length mismatch"
    );
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await?;
    let _partial = Partial(path.to_path_buf());
    let mut file = file;
    let mut hash = Sha256::new();
    let mut count = 0;
    loop {
        let mut next = Box::pin(response.chunk());
        let chunk = loop {
            ensure!(
                !progress.cancel.load(Ordering::Relaxed),
                "Download cancelled"
            );
            if let Ok(chunk) = tokio::time::timeout(Duration::from_millis(100), &mut next).await {
                break chunk?;
            }
        };
        let Some(chunk) = chunk else { break };
        count += chunk.len() as u64;
        ensure!(
            count <= size && count <= MAX_INSTALLER,
            "Installer exceeds expected size"
        );
        file.write_all(&chunk).await?;
        hash.update(&chunk);
        progress.bytes.store(count, Ordering::Relaxed);
    }
    ensure!(
        !progress.cancel.load(Ordering::Relaxed),
        "Download cancelled"
    );
    ensure!(
        count == size && format!("{:x}", hash.finalize()) == expected,
        "Installer SHA-256 mismatch"
    );
    file.sync_all().await?;
    drop(file);
    // Rename only verified, fully flushed content. The partial guard deletes failures.
    tokio::fs::rename(path, path.with_extension("exe")).await?;
    Ok(())
}
pub async fn download(
    root: PathBuf,
    release: updates::Release,
    progress: Arc<Progress>,
) -> Result<Prepared> {
    release.validate()?;
    numeric_version(&release.version)?;
    let api = format!(
        "https://api.github.com/repos/{REPO}/releases/tags/v{}",
        release.version
    );
    let metadata = updates::fetch_limited(&client(false)?, &api, updates::MAX_GITHUB_BYTES)
        .await?
        .context("Release is no longer available")?;
    let (bytes, sha256, url) = asset_metadata(&metadata, &release.version)?;
    ensure!(
        !progress.cancel.load(Ordering::Relaxed),
        "Download cancelled"
    );
    let folder = cache(&root)?.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&folder)?;
    let path = folder.join(installer_name(&release.version));
    progress.total.store(bytes, Ordering::Relaxed);
    progress.stage.store(1, Ordering::Relaxed);
    let response = client(true)?.get(url).send().await?;
    receive(
        response,
        &path.with_extension("part"),
        bytes,
        &sha256,
        &progress,
    )
    .await?;
    progress.stage.store(2, Ordering::Relaxed);
    let prepared = Prepared {
        version: release.version,
        path,
        bytes,
        sha256,
    };
    let check = prepared.clone();
    tokio::task::spawn_blocking(move || verified_installer(&check).map(drop)).await??;
    Ok(prepared)
}
fn hash(file: &mut File) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn verified_installer(prepared: &Prepared) -> Result<File> {
    numeric_version(&prepared.version)?;
    plain(&prepared.path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&prepared.path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() == prepared.bytes && meta.len() <= MAX_INSTALLER,
        "Installer length mismatch"
    );
    ensure!(
        hash(&mut file)? == prepared.sha256,
        "Installer SHA-256 mismatch"
    );
    Ok(file) // Prevent replacement/writes until Setup finishes.
}
/// All new agent/window starts and agent claims participate in this short coordination lock.
pub fn coordination(root: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("update.lock"))?;
    file.try_lock()
        .context("An application update is running")?;
    Ok(file)
}
pub fn startup_coordination(root: &Path) -> Result<File> {
    let start = Instant::now();
    loop {
        match coordination(root) {
            Ok(lock) => return Ok(lock),
            Err(error) if start.elapsed() >= Duration::from_secs(5) => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}
fn instance_lock(root: &Path, name: &str) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(name))?;
    file.try_lock()?;
    Ok(file)
}
#[derive(Serialize, Deserialize)]
struct Job {
    prepared: Prepared,
    target: PathBuf,
    app_hash: String,
    language: String,
}
fn same(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}
fn validate_folder(root: &Path, prepared: &Prepared) -> Result<PathBuf> {
    let folder = prepared.path.parent().context("Missing update folder")?;
    plain(folder)?;
    uuid::Uuid::parse_str(
        folder
            .file_name()
            .context("Missing folder name")?
            .to_str()
            .context("Invalid folder")?,
    )?;
    ensure!(
        same(
            folder.parent().context("Invalid update parent")?,
            &cache(root)?
        ),
        "Invalid update folder"
    );
    ensure!(
        prepared
            .path
            .file_name()
            .is_some_and(|n| n == installer_name(&prepared.version).as_str()),
        "Invalid installer filename"
    );
    Ok(folder.to_owned())
}
pub fn handoff(root: &Path, prepared: Prepared) -> Result<()> {
    let folder = validate_folder(root, &prepared)?;
    let _verified = verified_installer(&prepared)?;
    let exe = std::env::current_exe()?;
    let target = exe.parent().context("Missing installation folder")?;
    ensure!(
        target.join("unins000.exe").is_file(),
        "In-app updates require an installed copy. Install the full package first."
    );
    let helper = folder.join("StrajaBackupSQL-Updater.exe");
    std::fs::copy(&exe, &helper)?;
    // App-local CRT dependencies, never installed system-wide.
    for entry in std::fs::read_dir(target)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".dll")
            && ["vcruntime140", "msvcp140", "concrt140"]
                .iter()
                .any(|p| name.starts_with(p))
        {
            std::fs::copy(entry.path(), folder.join(entry.file_name()))?;
        }
    }
    let job = Job {
        prepared,
        target: target.into(),
        app_hash: hash(&mut File::open(&exe)?)?,
        language: crate::i18n::current().code().into(),
    };
    let job_path = folder.join("job.json");
    std::fs::write(&job_path, serde_json::to_vec(&job)?)?;
    let mut child = engine::command(helper)
        .current_dir(&folder)
        .arg("--apply-update")
        .arg(&job_path)
        .spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            bail!(
                "Update preparation stopped ({status}). See {}",
                folder.join("error.txt").display()
            );
        }
        if folder.join("ready").is_file() {
            std::fs::write(folder.join("commit"), b"install")?;
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(24 * 3600 + 90) {
            bail!("Update preparation timed out");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
fn installer_args(job: &Job) -> Result<Vec<String>> {
    let target = job.target.to_str().context("Invalid installation path")?;
    ensure!(
        job.target.is_absolute() && !target.contains(['"', '\r', '\n']),
        "Invalid installation path"
    );
    ensure!(
        ["en", "de", "es", "pt", "fr", "it"].contains(&job.language.as_str()),
        "Invalid language"
    );
    Ok(vec![
        "/SP-".into(),
        "/SILENT".into(),
        "/SUPPRESSMSGBOXES".into(),
        "/NORESTART".into(),
        "/NOCLOSEAPPLICATIONS".into(),
        "/NORESTARTAPPLICATIONS".into(),
        "/RESTARTEXITCODE=3010".into(),
        "/LOG".into(),
        format!("/DIR={target}"),
        format!("/LANG={}", job.language),
    ])
}
fn wait_lock(root: &Path, name: &str, timeout: Duration) -> Result<File> {
    let start = Instant::now();
    loop {
        match instance_lock(root, name) {
            Ok(lock) => return Ok(lock),
            Err(e) => {
                ensure!(
                    start.elapsed() < timeout,
                    "Timed out waiting for {name}: {e}"
                );
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}
pub fn apply(job_path: &Path) -> Result<()> {
    let root = store::data_dir()?;
    crate::i18n::reload(&root);
    let _update_lock = coordination(&root)?;
    ensure!(
        std::fs::metadata(job_path)?.len() <= 16384,
        "Update job too large"
    );
    let job: Job = serde_json::from_slice(&std::fs::read(job_path)?)?;
    let folder = validate_folder(&root, &job.prepared)?;
    ensure!(
        same(job_path, &folder.join("job.json"))
            && same(
                &std::env::current_exe()?,
                &folder.join("StrajaBackupSQL-Updater.exe")
            ),
        "Invalid update helper location"
    );
    let exe = job.target.join("StrajaBackupSQL.exe");
    plain(&job.target)?;
    ensure!(
        job.target.join("unins000.exe").is_file() && hash(&mut File::open(&exe)?)? == job.app_hash,
        "Installation changed; update cancelled"
    );
    let installer = verified_installer(&job.prepared)?;
    let args = installer_args(&job)?;
    let store = Store::open(&root)?;
    let was_running = instance_lock(&root, "agent.lock").is_err();
    if was_running {
        ensure!(
            store
                .get::<PathBuf>("agent", "executable")
                .is_ok_and(|path| same(&path, &job.target.join("StrajaBackupSQL-Agent.exe"))),
            "The running agent belongs to another installation. Restart this installation's agent first."
        );
    }
    let was_stopping = store.get::<bool>("agent", "stop").unwrap_or(false);
    let result = (|| -> Result<()> {
        // The coordination lock prevents new claims/startups. The current backup finishes normally.
        store.put("agent", "stop", &true)?;
        let _agent = wait_lock(&root, "agent.lock", Duration::from_secs(24 * 3600))?;
        std::fs::write(folder.join("ready"), b"ready")?;
        let start = Instant::now();
        while !folder.join("commit").is_file() {
            ensure!(
                start.elapsed() < Duration::from_secs(60),
                "Application did not confirm installation"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let _window = wait_lock(&root, "window.lock", Duration::from_secs(60))?;
        // File locks can be released just before Windows destroys the process/mutex.
        std::thread::sleep(Duration::from_secs(2));
        let status = engine::command(&job.prepared.path)
            .current_dir(&folder)
            .args(args)
            .status()?;
        ensure!(
            status.code() == Some(0),
            "Installer exited with code {:?}. See the Setup log in your TEMP folder.",
            status.code()
        );
        Ok(())
    })();
    drop(installer);
    let _ = store.put("agent", "stop", &was_stopping);
    if let Err(error) = &result {
        let _ = std::fs::write(folder.join("error.txt"), format!("{error:#}"));
    }
    drop(_update_lock);
    // On failure, restart the existing installation too; never leave scheduling stopped.
    if was_running && !was_stopping {
        let _ = engine::command(job.target.join("StrajaBackupSQL-Agent.exe")).spawn();
    }
    let _ = engine::command(exe).spawn();
    if result.is_ok() {
        let _ = std::fs::remove_file(&job.prepared.path);
    }
    result
}
pub fn notify_error(error: &anyhow::Error) {
    let message = format!(
        "{}\n\n{error:#}",
        tr("The update could not be completed. Your settings and backups are kept.")
    );
    let wide: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Straja Backup SQL\0".encode_utf16().collect();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
            std::ptr::null_mut(),
            wide.as_ptr(),
            title.as_ptr(),
            windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener};

    fn metadata() -> serde_json::Value {
        serde_json::json!({
            "tag_name":"v99.0.0", "draft":false, "prerelease":false,
            "html_url":format!("https://github.com/{REPO}/releases/tag/v99.0.0"),
            "assets":[{
                "name":installer_name("99.0.0"), "state":"uploaded", "size":10,
                "digest":format!("sha256:{}", "a".repeat(64)),
                "browser_download_url":format!("https://github.com/{REPO}/releases/download/v99.0.0/{}",installer_name("99.0.0"))
            }]
        })
    }
    #[test]
    fn metadata_requires_exact_product_stable_version_size_and_digest() {
        let good = metadata();
        assert!(asset_metadata(&serde_json::to_vec(&good).unwrap(), "99.0.0").is_ok());
        for (key, value) in [
            ("size", serde_json::json!(MAX_INSTALLER + 1)),
            ("size", serde_json::json!(0)),
            ("digest", serde_json::json!(null)),
            ("digest", serde_json::json!("sha256:wrong")),
            (
                "browser_download_url",
                serde_json::json!(
                    "https://github.com/weblikero/straja-backup/releases/download/v99.0.0/setup.exe"
                ),
            ),
        ] {
            let mut bad = good.clone();
            bad["assets"][0][key] = value;
            assert!(
                asset_metadata(&serde_json::to_vec(&bad).unwrap(), "99.0.0").is_err(),
                "{key}"
            );
        }
        for key in ["draft", "prerelease"] {
            let mut bad = good.clone();
            bad[key] = true.into();
            assert!(asset_metadata(&serde_json::to_vec(&bad).unwrap(), "99.0.0").is_err());
        }
        let mut duplicate = good.clone();
        duplicate["assets"]
            .as_array_mut()
            .unwrap()
            .push(good["assets"][0].clone());
        assert!(asset_metadata(&serde_json::to_vec(&duplicate).unwrap(), "99.0.0").is_err());
        assert!(asset_metadata(&serde_json::to_vec(&good).unwrap(), "98.0.0").is_err());
        assert!(numeric_version(updates::CURRENT_VERSION).is_err());
        assert!(numeric_version("99.0.0+build").is_err());
        for url in [
            "http://github.com/test",
            "https://github.com.evil.test/file",
            "https://user@github.com/file",
            "https://example.com/file",
        ] {
            assert!(!allowed_download(&reqwest::Url::parse(url).unwrap()));
        }
        assert!(allowed_download(
            &reqwest::Url::parse(
                "https://release-assets.githubusercontent.com/asset?signature=value"
            )
            .unwrap()
        ));
    }
    fn server(body: Vec<u8>, status: &str) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let status = status.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            let _ = stream.write_all(&body);
        });
        (url, handle)
    }
    #[test]
    fn streaming_download_checks_hash_size_cancel_and_keeps_only_verified_file() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let body = vec![42u8; 1024 * 1024];
            let hash = format!("{:x}", Sha256::digest(&body));
            for case in 0..5 {
                let (url, thread) = server(
                    body.clone(),
                    if case == 4 {
                        "503 Unavailable"
                    } else {
                        "200 OK"
                    },
                );
                let response = reqwest::Client::new().get(url).send().await.unwrap();
                let path = temp.path().join(format!("test{case}.part"));
                let progress = Progress::default();
                if case == 3 {
                    progress.cancel.store(true, Ordering::Relaxed);
                }
                let result = receive(
                    response,
                    &path,
                    body.len() as u64 + u64::from(case == 2),
                    if case == 1 { "wrong" } else { &hash },
                    &progress,
                )
                .await;
                thread.join().unwrap();
                assert_eq!(result.is_ok(), case == 0, "{case}");
                assert!(!path.exists());
                assert_eq!(path.with_extension("exe").exists(), case == 0);
            }
        });
    }
    #[test]
    fn installer_is_rehashed_and_locked_against_writes_and_coordination_is_exclusive() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(installer_name("99.0.0"));
        std::fs::write(&path, b"fixture").unwrap();
        let prepared = Prepared {
            version: "99.0.0".into(),
            path: path.clone(),
            bytes: 7,
            sha256: format!("{:x}", Sha256::digest(b"fixture")),
        };
        let locked = verified_installer(&prepared).unwrap();
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        drop(locked);
        std::fs::write(&path, b"changed").unwrap();
        assert!(verified_installer(&prepared).is_err());
        let lock = coordination(root.path()).unwrap();
        assert!(coordination(root.path()).is_err());
        drop(lock);
        assert!(coordination(root.path()).is_ok());
    }
}
