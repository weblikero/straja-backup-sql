//! Product-scoped update discovery. Installation is handled by the isolated updater.
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Utc};
use reqwest::{Client, Url};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

use crate::store::Store;

pub const FEED_URL: &str =
    "https://strajabackup.com/updates/strajabackupsql/windows-x64/stable.json";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GITHUB_LATEST: &str =
    "https://api.github.com/repos/weblikero/straja-backup-sql/releases/latest";
const MAX_FEED_BYTES: usize = 32 * 1024;
// GitHub includes metadata for every installer, notice and corresponding-source asset.
pub(crate) const MAX_GITHUB_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub automatic: bool,
    pub last_attempt: Option<DateTime<Utc>>,
    pub dismissed_version: Option<String>,
    pub last_release: Option<Release>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: true,
            last_attempt: None,
            dismissed_version: None,
            last_release: None,
        }
    }
}

impl Preferences {
    pub fn load(root: &Path) -> Self {
        Store::open(root)
            .and_then(|s| s.get("settings", "updates"))
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        Store::open(root)?.put("settings", "updates", self)
    }

    pub fn due(&self, now: DateTime<Utc>) -> bool {
        self.automatic
            && self
                .last_attempt
                .is_none_or(|last| now < last || now.signed_duration_since(last).num_hours() >= 24)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub schema_version: u32,
    pub product: String,
    pub platform: String,
    pub channel: String,
    pub version: String,
    pub release_url: String,
}

impl Release {
    pub fn validate(&self) -> Result<Version> {
        ensure!(self.schema_version == 1, "Unsupported update schema");
        ensure!(self.product == "strajabackupsql", "Different product");
        ensure!(self.platform == crate::platform::ID, "Different platform");
        ensure!(self.channel == "stable", "Different update channel");
        let version = Version::parse(&self.version)?;
        ensure!(
            version.pre.is_empty(),
            "Prereleases are not offered on the stable channel"
        );
        let url = Url::parse(&self.release_url)?;
        ensure!(
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Unsafe release URL"
        );
        // This exact public repository belongs to this product, not the suite's file backup app.
        let github = url.host_str() == Some("github.com")
            && url.path()
                == format!(
                    "/weblikero/straja-backup-sql/releases/tag/v{}",
                    self.version
                );
        let website = matches!(
            url.host_str(),
            Some("strajabackup.com" | "www.strajabackup.com")
        ) && matches!(
            url.path(),
            "/downloads/strajabackupsql" | "/downloads/strajabackupsql/"
        );
        ensure!(
            github || website,
            "Release URL is outside the product's official pages"
        );
        Ok(version)
    }

    pub fn is_newer_than(&self, current: &str) -> Result<bool> {
        Ok(self
            .validate()?
            .cmp_precedence(&Version::parse(current)?)
            .is_gt())
    }
}

#[derive(Debug, Clone)]
pub enum CheckResult {
    Available(Release),
    Current,
    NotPublished,
}

pub fn parse_feed(bytes: &[u8], current: &str) -> Result<CheckResult> {
    ensure!(bytes.len() <= MAX_FEED_BYTES, "Update feed is too large");
    let release: Release = serde_json::from_slice(bytes)?;
    if release.is_newer_than(current)? {
        Ok(CheckResult::Available(release))
    } else {
        Ok(CheckResult::Current)
    }
}

async fn fetch_bytes(client: &Client, url: &str) -> Result<Option<Vec<u8>>> {
    fetch_limited(client, url, MAX_FEED_BYTES).await
}

pub(crate) async fn fetch_limited(
    client: &Client,
    url: &str,
    limit: usize,
) -> Result<Option<Vec<u8>>> {
    let mut response = client
        .get(url)
        .header("Accept", "application/json")
        .header("Cache-Control", "no-cache")
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    // Redirects are deliberately not followed: only this product's HTTPS feed is authoritative.
    ensure!(
        response.status().is_success(),
        "Update server returned {}",
        response.status()
    );
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "Update feed is too large"
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > limit {
            bail!("Update feed is too large");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

#[derive(Deserialize)]
pub(crate) struct GithubRelease {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    pub(crate) assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
pub(crate) struct GithubAsset {
    pub(crate) name: String,
    pub(crate) size: u64,
    state: String,
    #[serde(default)]
    pub(crate) digest: Option<String>,
    #[serde(default)]
    pub(crate) browser_download_url: String,
}

pub(crate) fn parse_github(bytes: &[u8], current: &str) -> Result<CheckResult> {
    ensure!(
        bytes.len() <= MAX_GITHUB_BYTES,
        "Release information is too large"
    );
    let github: GithubRelease = serde_json::from_slice(bytes)?;
    ensure!(
        !github.draft && !github.prerelease,
        "Not a stable public release"
    );
    let version = github
        .tag_name
        .strip_prefix('v')
        .ok_or_else(|| anyhow::anyhow!("Invalid release tag"))?;
    let release = Release {
        schema_version: 1,
        product: "strajabackupsql".into(),
        platform: crate::platform::ID.into(),
        channel: "stable".into(),
        version: version.into(),
        release_url: github.html_url,
    };
    if !release.is_newer_than(current)? {
        return Ok(CheckResult::Current);
    }
    let filename = crate::platform::artifact(version);
    if !github.assets.iter().any(|asset| {
        asset.name == filename && asset.size > 0 && asset.state == "uploaded"
    }) {
        return Ok(CheckResult::NotPublished);
    }
    Ok(CheckResult::Available(release))
}

pub async fn check() -> Result<CheckResult> {
    let client = Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("StrajaBackupSQL/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let feed = format!(
        "https://strajabackup.com/updates/strajabackupsql/{}/stable.json",
        crate::platform::ID
    );
    check_sources(&client, GITHUB_LATEST, &feed, CURRENT_VERSION).await
}

async fn check_sources(
    client: &Client,
    github_url: &str,
    website_url: &str,
    current: &str,
) -> Result<CheckResult> {
    // GitHub is authoritative. An old website feed must not hide a new GitHub release.
    let github_result = match fetch_limited(client, github_url, MAX_GITHUB_BYTES).await {
        Ok(Some(bytes)) => parse_github(&bytes, current).map(Some),
        Ok(None) => Ok(None),
        Err(error) => Err(error),
    };
    match github_result {
        Ok(Some(result)) => Ok(result),
        github_result => match fetch_bytes(client, website_url).await? {
            Some(bytes) => parse_feed(&bytes, current),
            None => match github_result {
                Ok(None) => Ok(CheckResult::NotPublished),
                Err(error) => Err(error),
                _ => unreachable!(),
            },
        },
    }
}

pub fn open_release(release: &Release) -> Result<()> {
    release.validate()?;
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let action: Vec<u16> = "open\0".encode_utf16().collect();
        let url: Vec<u16> = release.release_url.encode_utf16().chain(Some(0)).collect();
        // No command shell; Windows opens the validated HTTPS URL in the user's browser.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                action.as_ptr(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        ensure!((result as isize) > 32, "Could not open the browser");
        Ok(())
    }
    #[cfg(unix)]
    crate::platform::open(Path::new(&release.release_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Release {
        Release {
            schema_version: 1,
            product: "strajabackupsql".into(),
            platform: crate::platform::ID.into(),
            channel: "stable".into(),
            version: version.into(),
            release_url: format!(
                "https://github.com/weblikero/straja-backup-sql/releases/tag/v{version}"
            ),
        }
    }

    #[test]
    fn semantic_order_never_offers_downgrades_or_build_metadata_as_updates() {
        assert!(release("0.1.10").is_newer_than("0.1.9").unwrap());
        assert!(!release("0.1.9").is_newer_than("0.1.10").unwrap());
        assert!(!release("0.1.7").is_newer_than("0.1.7").unwrap());
        assert!(
            !release("0.1.7+build2")
                .is_newer_than("0.1.7+build1")
                .unwrap()
        );
        assert!(release("0.1.8-beta.1").validate().is_err());
        assert!(release("latest").validate().is_err());
    }

    #[test]
    fn rejects_other_products_platforms_channels_and_untrusted_urls() {
        let good = release("0.1.8");
        for (key, value) in [
            ("product", "strajabackup"),
            ("platform", "windows-arm64"),
            ("channel", "preview"),
        ] {
            let mut json = serde_json::to_value(&good).unwrap();
            json[key] = value.into();
            assert!(parse_feed(&serde_json::to_vec(&json).unwrap(), CURRENT_VERSION).is_err());
        }
        for url in [
            "http://strajabackup.com/downloads/strajabackupsql/",
            "https://strajabackup.com.evil.test/downloads/strajabackupsql/",
            "https://user@strajabackup.com/downloads/strajabackupsql/",
            "https://strajabackup.com/downloads/strajabackup/",
            "https://strajabackup.com/downloads/strajabackupsql/?redirect=evil",
            "https://github.com/weblikero/straja-backup/releases/tag/v0.1.8",
            "https://github.com/other/straja-backup-sql/releases/tag/v0.1.8",
            "file:///C:/Windows/system32/cmd.exe",
        ] {
            let mut bad = good.clone();
            bad.release_url = url.into();
            assert!(bad.validate().is_err(), "{url}");
        }
        let mut bad = good.clone();
        bad.schema_version = 2;
        assert!(bad.validate().is_err());
        assert!(parse_feed(b"<html>Error</html>", CURRENT_VERSION).is_err());
        assert!(parse_feed(&vec![b' '; MAX_FEED_BYTES + 1], CURRENT_VERSION).is_err());
        let mut website = good;
        website.release_url = "https://strajabackup.com/downloads/strajabackupsql/".into();
        assert!(website.validate().is_ok());
    }

    #[test]
    fn check_interval_persists_and_recovers_from_clock_changes() {
        let root = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let mut prefs = Preferences::default();
        assert!(prefs.due(now));
        prefs.last_attempt = Some(now);
        prefs.dismissed_version = Some("0.1.8".into());
        prefs.save(root.path()).unwrap();
        prefs = Preferences::load(root.path());
        assert_eq!(prefs.dismissed_version.as_deref(), Some("0.1.8"));
        assert!(!prefs.due(now + chrono::Duration::hours(23)));
        assert!(prefs.due(now + chrono::Duration::hours(24)));
        assert!(prefs.due(now - chrono::Duration::hours(1)));
        prefs.automatic = false;
        assert!(!prefs.due(now + chrono::Duration::days(2)));
    }

    #[test]
    fn http_response_handling_with_a_local_fixture_only() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let valid = serde_json::to_string(&release("0.1.10")).unwrap();
        for (status, body, expected) in [
            ("200 OK", valid, 1),
            ("404 Not Found", String::new(), 2),
            ("503 Unavailable", String::new(), 0),
            ("302 Found", String::new(), 0),
            ("200 OK", "x".repeat(MAX_FEED_BYTES + 1), 0),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let result = runtime.block_on(async {
                // HTTP is permitted only by this local test client, never by check().
                let client = Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(5))
                    .build()
                    .unwrap();
                match fetch_bytes(&client, &url).await? {
                    Some(bytes) => parse_feed(&bytes, "0.1.9"),
                    None => Ok(CheckResult::NotPublished),
                }
            });
            server.join().unwrap();
            match expected {
                1 => assert!(matches!(result.unwrap(), CheckResult::Available(_))),
                2 => assert!(matches!(result.unwrap(), CheckResult::NotPublished)),
                _ => assert!(result.is_err()),
            }
        }
    }

    #[test]
    fn github_priority_and_website_fallback_use_local_http_only() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let github = serde_json::json!({"tag_name":"v0.1.11", "draft":false, "prerelease":false,
            "html_url":"https://github.com/weblikero/straja-backup-sql/releases/tag/v0.1.11",
            "assets":[{"name":crate::platform::artifact("0.1.11"),"size":100,"state":"uploaded"}]}).to_string();
        let website = serde_json::to_string(&release("0.1.12")).unwrap();
        for (responses, expected) in [
            (vec![(200, github.clone())], Some("0.1.11")),
            (
                vec![(200, github.replace("0.1.11", "0.1.9"))],
                Some("current"),
            ),
            (
                vec![(404, String::new()), (200, website.clone())],
                Some("0.1.12"),
            ),
            (
                vec![(200, "invalid".into()), (200, website)],
                Some("0.1.12"),
            ),
            (vec![(404, String::new()), (404, String::new())], None),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                for (status, body) in responses {
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(
                                    std::time::Instant::now() < deadline,
                                    "Missing source request"
                                );
                                std::thread::sleep(Duration::from_millis(10));
                            }
                            Err(e) => panic!("{e}"),
                        }
                    };
                    stream
                        .set_nonblocking(false)
                        .unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0u8; 1];
                        stream.read_exact(&mut byte).unwrap();
                        request.push(byte[0]);
                        assert!(request.len() < 8192);
                    }
                    write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            });
            let client = Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap();
            let result = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(check_sources(&client, &url, &url, "0.1.10"));
            server.join().unwrap();
            match (expected, result.unwrap()) {
                (Some("current"), CheckResult::Current) => {}
                (Some(version), CheckResult::Available(release)) => {
                    assert_eq!(release.version, version)
                }
                (None, CheckResult::NotPublished) => {}
                other => panic!("Unexpected source resolution: {other:?}"),
            }
        }
    }

    #[test]
    fn github_requires_a_stable_release_with_the_correct_installer() {
        let good = serde_json::json!({"tag_name":"v0.1.10", "draft":false, "prerelease":false,
            "html_url":"https://github.com/weblikero/straja-backup-sql/releases/tag/v0.1.10",
            "assets":[{"name":crate::platform::artifact("0.1.10"),"size":100,"state":"uploaded"}]});
        assert!(matches!(
            parse_github(&serde_json::to_vec(&good).unwrap(), "0.1.9").unwrap(),
            CheckResult::Available(_)
        ));
        let mut with_notes = good.clone();
        with_notes["body"] = "Release metadata".repeat(3000).into();
        assert!(parse_github(&serde_json::to_vec(&with_notes).unwrap(), "0.1.9").is_ok());
        assert!(parse_github(&vec![b' '; MAX_GITHUB_BYTES + 1], "0.1.9").is_err());
        for key in ["draft", "prerelease"] {
            let mut bad = good.clone();
            bad[key] = true.into();
            assert!(parse_github(&serde_json::to_vec(&bad).unwrap(), "0.1.9").is_err());
        }
        let mut bad = good.clone();
        bad["assets"][0]["name"] = "StrajaBackup-0.1.10-setup.exe".into();
        assert!(matches!(parse_github(&serde_json::to_vec(&bad).unwrap(), "0.1.9").unwrap(), CheckResult::NotPublished));
        let mut bad = good;
        bad["assets"][0]["state"] = "new".into();
        assert!(matches!(parse_github(&serde_json::to_vec(&bad).unwrap(), "0.1.9").unwrap(), CheckResult::NotPublished));
    }
}
