use crate::i18n::{self, tr};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Engine {
    MariaDb,
    MySql,
    PostgreSql,
}
impl Engine {
    pub const ALL: [Self; 3] = [Self::MariaDb, Self::MySql, Self::PostgreSql];
    pub fn tools_folder(self) -> &'static str {
        match self {
            Self::MariaDb => "mariadb",
            Self::MySql => "mysql",
            Self::PostgreSql => "postgresql",
        }
    }
    pub fn bundled_version(self) -> String {
        if !cfg!(windows) {
            return crate::platform::tools_root()
                .ok()
                .and_then(|p| std::fs::read(p.join("versions.json")).ok())
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v[self.tools_folder()].as_str().map(str::to_owned))
                .unwrap_or_else(|| "—".into());
        }
        match self {
            Self::MariaDb => "11.4.10",
            Self::MySql => "8.4.11",
            Self::PostgreSql => "18.6",
        }
        .into()
    }
    pub fn port(self) -> u16 {
        if self == Self::PostgreSql { 5432 } else { 3306 }
    }
    pub fn client(self) -> &'static str {
        match self {
            Self::MariaDb => {
                if cfg!(windows) {
                    "mariadb.exe"
                } else {
                    "mariadb"
                }
            }
            Self::MySql => {
                if cfg!(windows) {
                    "mysql.exe"
                } else {
                    "mysql"
                }
            }
            Self::PostgreSql => {
                if cfg!(windows) {
                    "psql.exe"
                } else {
                    "psql"
                }
            }
        }
    }
    pub fn dump(self) -> &'static str {
        match self {
            Self::MariaDb => {
                if cfg!(windows) {
                    "mariadb-dump.exe"
                } else {
                    "mariadb-dump"
                }
            }
            Self::MySql => {
                if cfg!(windows) {
                    "mysqldump.exe"
                } else {
                    "mysqldump"
                }
            }
            Self::PostgreSql => {
                if cfg!(windows) {
                    "pg_dump.exe"
                } else {
                    "pg_dump"
                }
            }
        }
    }
}
impl fmt::Display for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MariaDb => "MariaDB",
            Self::MySql => "MySQL",
            Self::PostgreSql => "PostgreSQL",
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub tools: PathBuf,
    pub tls: bool,
    pub ca: String,
}
impl Connection {
    /// Empty means app-local tools; never search PATH or installed database products.
    pub fn tools_dir(&self) -> Result<PathBuf> {
        if !self.tools.as_os_str().is_empty() {
            if !self.tools.is_absolute() {
                bail!(
                    "{}",
                    tr("The custom tools folder must use an absolute path.")
                );
            }
            return Ok(self.tools.clone());
        }
        let executable = std::env::current_exe()?;
        let root = executable
            .parent()
            .context(tr("Cannot determine the application location."))?;
        Ok(root
            .join("tools")
            .join(self.engine.tools_folder())
            .join("bin"))
    }
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.id)?;
        for (label, value) in [
            (tr("Name"), &self.name),
            (tr("Server"), &self.host),
            (tr("Username"), &self.user),
        ] {
            if value.trim().is_empty() || value.len() > 255 || value.chars().any(char::is_control) {
                bail!(
                    "{}",
                    i18n::format("{0}: invalid value.", &[(label).to_string()])
                );
            }
        }
        if self.port == 0 {
            bail!("{}", tr("Port must be between 1 and 65535."));
        }
        let tools = self.tools_dir()?;
        if !tools.join(self.engine.client()).is_file() || !tools.join(self.engine.dump()).is_file()
        {
            bail!(
                "{}",
                i18n::format(
                    "Missing {0} or {1} in {2}. Reinstall the complete package or select custom tools in advanced options.",
                    &[
                        (self.engine.client()).to_string(),
                        (self.engine.dump()).to_string(),
                        (tools.display()).to_string()
                    ]
                )
            );
        }
        if !self.ca.is_empty() && !PathBuf::from(&self.ca).is_file() {
            bail!("{}", tr("CA certificate does not exist."));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frequency {
    Manual,
    Minutes,
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Yearly,
    Once,
}
impl Frequency {
    pub const ALL: [Self; 8] = [
        Self::Manual,
        Self::Minutes,
        Self::Hourly,
        Self::Daily,
        Self::Weekly,
        Self::Monthly,
        Self::Yearly,
        Self::Once,
    ];
}
impl fmt::Display for Frequency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Manual => tr("Manual"),
            Self::Minutes => tr("Every N minutes"),
            Self::Hourly => tr("Hourly"),
            Self::Daily => tr("Daily"),
            Self::Weekly => tr("Weekly"),
            Self::Monthly => tr("Monthly"),
            Self::Yearly => tr("Yearly"),
            Self::Once => tr("Once"),
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub frequency: Frequency,
    pub minutes: u32,
    pub hour: u32,
    pub minute: u32,
    pub weekday: u32,
    pub day: u32,
    pub month: u32,
    pub date: NaiveDate,
    pub timezone: String,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            frequency: Frequency::Daily,
            minutes: 60,
            hour: 2,
            minute: 0,
            weekday: 0,
            day: 1,
            month: 1,
            date: Utc::now().date_naive(),
            timezone: "UTC".into(),
        }
    }
}
impl Schedule {
    pub fn validate(&self) -> Result<()> {
        let _: chrono_tz::Tz = self.timezone.parse()?;
        if self.hour > 23
            || self.minute > 59
            || self.weekday > 6
            || !(1..=31).contains(&self.day)
            || !(1..=12).contains(&self.month)
            || !(1..=525600).contains(&self.minutes)
        {
            bail!(
                "{}",
                tr("Invalid schedule. Check the time, interval and date.")
            );
        }
        Ok(())
    }
    pub fn next_after(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.validate().ok()?;
        let tz: chrono_tz::Tz = self.timezone.parse().ok()?;
        match self.frequency {
            Frequency::Manual => return None,
            Frequency::Minutes => return Some(now + Duration::minutes(self.minutes.into())),
            Frequency::Hourly => return Some(now + Duration::hours(1)),
            _ => {}
        }
        let today = now.with_timezone(&tz).date_naive();
        if self.frequency == Frequency::Once {
            return tz
                .from_local_datetime(&self.date.and_hms_opt(self.hour, self.minute, 0)?)
                .earliest()
                .map(|t| t.with_timezone(&Utc))
                .filter(|t| *t > now);
        }
        for offset in 0..=370 {
            let date = today + Duration::days(offset);
            let last_day = (28..=31)
                .rev()
                .find(|d| NaiveDate::from_ymd_opt(date.year(), date.month(), *d).is_some())?;
            let matches = match self.frequency {
                Frequency::Daily => true,
                Frequency::Weekly => date.weekday().num_days_from_monday() == self.weekday,
                Frequency::Monthly => date.day() == self.day.min(last_day),
                Frequency::Yearly => {
                    date.month() == self.month && date.day() == self.day.min(last_day)
                }
                Frequency::Once => date == self.date,
                _ => false,
            };
            if matches {
                // Skip nonexistent local times; choose only the first repeated DST occurrence.
                if let Some(t) = tz
                    .from_local_datetime(&date.and_hms_opt(self.hour, self.minute, 0)?)
                    .earliest()
                    && t.with_timezone(&Utc) > now
                {
                    return Some(t.with_timezone(&Utc));
                }
            }
        }
        None
    }
    pub fn label(&self) -> String {
        match self.frequency {
            Frequency::Manual => tr("Manual").into(),
            Frequency::Minutes => i18n::format("Every {0} minutes", &[(self.minutes).to_string()]),
            Frequency::Hourly => tr("Every hour").into(),
            _ => format!(
                "{} · {:02}:{:02} · {}",
                self.frequency, self.hour, self.minute, self.timezone
            ),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compression {
    None,
    Gzip,
}
impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if *self == Self::Gzip {
            tr("GZIP · compressed")
        } else {
            tr("No compression")
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub connection: String,
    pub databases: Vec<String>,
    pub destination: PathBuf,
    #[serde(default)]
    pub remote: Option<crate::destinations::Remote>,
    pub schedule: Schedule,
    pub compression: Compression,
    pub keep_count: Option<u32>,
    pub keep_days: Option<u32>,
    pub enabled: bool,
    pub next_due: Option<DateTime<Utc>>,
    pub notify: bool,
    pub timeout_minutes: u64,
}
impl Plan {
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.id)?;
        uuid::Uuid::parse_str(&self.connection)?;
        if let Some(remote) = &self.remote {
            remote.validate()?;
        }
        if self.name.trim().is_empty() || self.name.len() > 120 {
            bail!("{}", tr("Enter a plan name (up to 120 characters)."));
        }
        if self.databases.is_empty() || self.databases.len() > 500 {
            bail!("{}", tr("Select between 1 and 500 databases."));
        }
        for db in &self.databases {
            validate_database(db)?;
        }
        if !self.destination.is_absolute() || !self.destination.is_dir() {
            bail!(
                "{}",
                tr("Choose an existing backup folder with an absolute path.")
            );
        }
        if self.keep_count.is_some_and(|v| v == 0 || v > 100000)
            || self.keep_days.is_some_and(|v| v == 0 || v > 36500)
        {
            bail!(
                "{}",
                tr("Retention must be positive; leave blank for unlimited.")
            );
        }
        if !(1..=1440).contains(&self.timeout_minutes) {
            bail!(
                "{}",
                tr("Export time limit must be between 1 and 1440 minutes.")
            );
        }
        self.schedule.validate()
    }
}
pub fn validate_database(db: &str) -> Result<()> {
    if db.trim().is_empty()
        || db.len() > 255
        || db.starts_with('-')
        || db.contains('=')
        || db.contains("://")
        || db.chars().any(char::is_control)
    {
        bail!(
            "{}",
            tr("Invalid database name (options and connection strings are not allowed).")
        );
    }
    Ok(())
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MailSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub from: String,
    pub to: String,
    pub implicit_tls: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupProgress {
    pub phase: String,
    pub database: String,
    pub database_index: usize,
    pub database_count: usize,
    pub bytes: u64,
    pub total_bytes: Option<u64>,
    pub updated: DateTime<Utc>,
}
impl BackupProgress {
    pub fn percent(&self) -> Option<f32> {
        self.total_bytes
            .filter(|n| *n > 0)
            .map(|n| (self.bytes as f64 / n as f64 * 100.).min(100.) as f32)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    pub kind: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub status: String,
    pub started: String,
    pub finished: Option<String>,
    pub detail: String,
    pub folder: Option<PathBuf>,
    #[serde(default)]
    pub progress: Option<BackupProgress>,
}
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub connections: Vec<Connection>,
    pub plans: Vec<Plan>,
    pub runs: Vec<Run>,
    pub heartbeat: Option<DateTime<Utc>>,
    pub mail: MailSettings,
}
