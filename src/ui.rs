use chrono::{NaiveDate, Utc};
use iced::{
    Color, Element, Fill, Length, Task, Theme,
    widget::{
        Space, button, checkbox, column, container, pick_list, progress_bar, row, scrollable, text,
        text_input,
    },
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use straja_sql::destinations::{self, Authorized, Provider, Remote};
use straja_sql::i18n::{self, Language, tr};
use straja_sql::{
    engine, mail,
    model::*,
    store::{self, Store},
    updater, updates, worker,
};
use zeroize::{Zeroize, Zeroizing};

const INK: Color = Color::from_rgb8(25, 40, 61);
const MUTED: Color = Color::from_rgb8(107, 121, 139);
const NAVY: Color = Color::from_rgb8(16, 29, 48);
const GREEN: Color = Color::from_rgb8(16, 133, 111);
const BG: Color = Color::from_rgb8(243, 246, 249);
const LINE: Color = Color::from_rgb8(225, 232, 239);

pub fn run() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .executor::<straja_sql::resources::UiExecutor>()
        .title(concat!(
            "StrajaBackupSQL v",
            env!("CARGO_PKG_VERSION"),
            " · Database Backup Studio"
        ))
        .theme(|_: &App| {
            Theme::custom(
                "StrajaBackupSQL",
                iced::theme::Palette {
                    background: BG,
                    text: INK,
                    primary: GREEN,
                    success: GREEN,
                    danger: Color::from_rgb8(196, 65, 67),
                    warning: Color::from_rgb8(190, 126, 39),
                },
            )
        })
        .subscription(|app: &App| {
            let active = app
                .state
                .runs
                .iter()
                .any(|r| matches!(r.status.as_str(), "În lucru" | "În așteptare"));
            let refresh =
                iced::time::every(Duration::from_secs(if app.update_progress.is_some() {
                    1
                } else if active {
                    2
                } else {
                    5
                }))
                .map(|_| Msg::Tick);
            let focus = iced::window::events().map(|(_, event)| Msg::WindowEvent(event));
            if app.focused
                && app.page == Page::History
                && app.state.runs.iter().any(|run| {
                    run.status == "În lucru"
                        && run.progress.as_ref().is_none_or(|p| p.percent().is_none())
                })
            {
                iced::Subscription::batch([
                    refresh,
                    focus,
                    iced::time::every(Duration::from_millis(250)).map(|_| Msg::Animate),
                ])
            } else {
                iced::Subscription::batch([refresh, focus])
            }
        })
        .window(iced::window::Settings {
            size: (1280., 840.).into(),
            min_size: Some((1000., 700.).into()),
            icon: iced::window::icon::from_rgba(
                include_bytes!("../installer/assets/logo.rgba").to_vec(),
                128,
                128,
            )
            .ok(),
            ..Default::default()
        })
        .default_font(iced::Font::with_name("Segoe UI"))
        .run()
}
#[derive(Debug, Clone, Copy, PartialEq)]
enum Page {
    Dashboard,
    Connections,
    Plans,
    History,
    Tables,
    Settings,
    Help,
    ConnectionForm,
    PlanForm,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    id: String,
    name: String,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}
#[derive(Debug, Clone)]
enum Msg {
    Tick,
    Animate,
    WindowEvent(iced::window::Event),
    Language(Language),
    Page(Page),
    NewConnection,
    EditConnection(String),
    NewPlan,
    EditPlan(String),
    Field(&'static str, String),
    Engine(Engine),
    Frequency(Frequency),
    Compression(Compression),
    DestinationProvider(Provider),
    DestinationTls(bool),
    AuthorizeDestination,
    DestinationAuthorized(String, Result<Authorized, String>),
    TestDestination,
    Tls(bool),
    CustomTools(bool),
    Notify(bool),
    ImplicitTls(bool),
    SelectConnection(Choice),
    SelectDb(String, bool),
    Browse(&'static str),
    Browsed(&'static str, Option<PathBuf>),
    SaveConnection,
    SavePlan,
    Discover(String),
    ToggleExplorer(String),
    SelectDatabase(String, String),
    TablesLoaded(u64, Result<Vec<TableInfo>, String>),
    TablePage(usize),
    TestForm,
    Discovered(String, Result<Vec<String>, String>),
    Run(String),
    Toggle(String),
    AskDelete(&'static str, String),
    Delete,
    CancelDelete,
    LaunchAgent,
    StopAgent,
    SaveMail,
    TestMail,
    Done(Result<String, String>),
    Verify(PathBuf),
    OpenFolder(PathBuf),
    CheckUpdates(bool),
    UpdatesChecked(Result<updates::CheckResult, String>),
    AutomaticUpdates(bool),
    DismissUpdate,
    DownloadUpdate,
    CancelDownload,
    UpdateDownloaded(Result<updater::Prepared, String>),
    UpdateHandoff(Result<(), String>),
}
#[derive(Default)]
enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Current,
    NotPublished,
    Failed,
}
struct ConnectionForm {
    id: String,
    name: String,
    engine: Engine,
    host: String,
    port: String,
    user: String,
    password: String,
    tools: String,
    custom_tools: bool,
    tls: bool,
    ca: String,
}
impl Default for ConnectionForm {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            engine: Engine::MariaDb,
            host: String::new(),
            port: "3306".into(),
            user: "backup_reader".into(),
            password: String::new(),
            tools: String::new(),
            custom_tools: false,
            tls: true,
            ca: String::new(),
        }
    }
}
impl ConnectionForm {
    fn model(&self) -> anyhow::Result<Connection> {
        Ok(Connection {
            id: self.id.clone(),
            name: self.name.trim().into(),
            engine: self.engine,
            host: self.host.trim().into(),
            port: self.port.parse()?,
            user: self.user.clone(),
            tools: if self.custom_tools {
                if self.tools.trim().is_empty() {
                    anyhow::bail!(
                        "{}",
                        tr("Choose a custom tools folder or use the bundled tools.")
                    );
                }
                PathBuf::from(self.tools.trim())
            } else {
                PathBuf::new()
            },
            tls: self.tls,
            ca: self.ca.clone(),
        })
    }
    fn from(c: &Connection) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            engine: c.engine,
            host: c.host.clone(),
            port: c.port.to_string(),
            user: c.user.clone(),
            password: String::new(),
            tools: c.tools.to_string_lossy().into(),
            custom_tools: !c.tools.as_os_str().is_empty(),
            tls: c.tls,
            ca: c.ca.clone(),
        }
    }
}
struct PlanForm {
    id: String,
    name: String,
    connection: Option<Choice>,
    databases: Vec<String>,
    manual_dbs: String,
    destination: String,
    remote: Remote,
    remote_port: String,
    remote_bandwidth: String,
    remote_secret: Zeroizing<String>,
    remote_saved: bool,
    frequency: Frequency,
    time: String,
    minutes: String,
    weekday: String,
    day: String,
    month: String,
    date: String,
    timezone: String,
    compression: Compression,
    count: String,
    days: String,
    notify: bool,
    timeout: String,
    enabled: bool,
}
impl Default for PlanForm {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            connection: None,
            databases: vec![],
            manual_dbs: String::new(),
            destination: String::new(),
            remote: Remote::default(),
            remote_port: "21".into(),
            remote_bandwidth: "0".into(),
            remote_secret: Zeroizing::new(String::new()),
            remote_saved: false,
            frequency: Frequency::Daily,
            time: "02:00".into(),
            minutes: "60".into(),
            weekday: "1".into(),
            day: "1".into(),
            month: "1".into(),
            date: Utc::now().format("%Y-%m-%d").to_string(),
            timezone: "UTC".into(),
            compression: Compression::Gzip,
            count: String::new(),
            days: String::new(),
            notify: false,
            timeout: "120".into(),
            enabled: true,
        }
    }
}
impl PlanForm {
    fn remote_model(&self) -> anyhow::Result<Remote> {
        let r = Remote {
            port: self.remote_port.parse()?,
            bandwidth_mib: self.remote_bandwidth.parse()?,
            ..self.remote.clone()
        };
        r.validate()?;
        Ok(r)
    }
    fn model(&self) -> anyhow::Result<Plan> {
        let (h, m) = self
            .time
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("{}", tr("Time must use HH:MM format")))?;
        let weekday: u32 = self.weekday.parse()?;
        anyhow::ensure!(
            (1..=7).contains(&weekday),
            "{}",
            tr("Weekday: 1 (Monday) – 7 (Sunday).")
        );
        let schedule = Schedule {
            frequency: self.frequency,
            minutes: self.minutes.parse()?,
            hour: h.parse()?,
            minute: m.parse()?,
            weekday: weekday - 1,
            day: self.day.parse()?,
            month: self.month.parse()?,
            date: NaiveDate::parse_from_str(&self.date, "%Y-%m-%d")?,
            timezone: self.timezone.trim().into(),
        };
        schedule.validate()?;
        anyhow::ensure!(
            schedule.frequency != Frequency::Once || schedule.next_after(Utc::now()).is_some(),
            "{}",
            tr("Choose a valid future date and time in the selected time zone.")
        );
        let optional = |s: &str| -> anyhow::Result<Option<u32>> {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(s.parse()?))
            }
        };
        let mut databases = self.databases.clone();
        for db in self
            .manual_dbs
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !databases.iter().any(|d| d == db) {
                databases.push(db.into());
            }
        }
        Ok(Plan {
            id: self.id.clone(),
            name: self.name.trim().into(),
            connection: self
                .connection
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("{}", tr("Choose a connection.")))?
                .id
                .clone(),
            databases,
            destination: PathBuf::from(&self.destination),
            remote: if self.remote.provider == Provider::Local {
                None
            } else {
                Some(self.remote_model()?)
            },
            next_due: schedule.next_after(Utc::now()),
            schedule,
            compression: self.compression,
            keep_count: optional(&self.count)?,
            keep_days: optional(&self.days)?,
            enabled: self.enabled,
            notify: self.notify,
            timeout_minutes: self.timeout.parse()?,
        })
    }
    fn from(p: &Plan, s: &Snapshot) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            connection: s
                .connections
                .iter()
                .find(|c| c.id == p.connection)
                .map(|c| Choice {
                    id: c.id.clone(),
                    name: c.name.clone(),
                }),
            databases: p.databases.clone(),
            manual_dbs: String::new(),
            destination: p.destination.to_string_lossy().into(),
            remote: p.remote.clone().unwrap_or_default(),
            remote_port: p.remote.as_ref().map(|r| r.port).unwrap_or(21).to_string(),
            remote_bandwidth: p
                .remote
                .as_ref()
                .map(|r| r.bandwidth_mib)
                .unwrap_or(0)
                .to_string(),
            remote_secret: Zeroizing::new(String::new()),
            remote_saved: p.remote.is_some(),
            frequency: p.schedule.frequency,
            time: format!("{:02}:{:02}", p.schedule.hour, p.schedule.minute),
            minutes: p.schedule.minutes.to_string(),
            weekday: (p.schedule.weekday + 1).to_string(),
            day: p.schedule.day.to_string(),
            month: p.schedule.month.to_string(),
            date: p.schedule.date.to_string(),
            timezone: p.schedule.timezone.clone(),
            compression: p.compression,
            count: p.keep_count.map(|v| v.to_string()).unwrap_or_default(),
            days: p.keep_days.map(|v| v.to_string()).unwrap_or_default(),
            notify: p.notify,
            timeout: p.timeout_minutes.to_string(),
            enabled: p.enabled,
        }
    }
}
struct App {
    root: PathBuf,
    monitor: Option<Store>,
    focused: bool,
    update_preferences: updates::Preferences,
    update_status: UpdateStatus,
    update_progress: Option<Arc<updater::Progress>>,
    state: Snapshot,
    page: Page,
    connection: ConnectionForm,
    plan: PlanForm,
    dbs: HashMap<String, Vec<String>>,
    expanded: HashSet<String>,
    selected_database: Option<(String, String)>,
    tables: Vec<TableInfo>,
    tables_loading: bool,
    tables_error: String,
    tables_request: u64,
    table_filter: String,
    table_page: usize,
    notice: String,
    error: bool,
    busy: bool,
    delete: Option<(&'static str, String)>,
    mail: MailSettings,
    mail_port: String,
    mail_password: String,
}
impl App {
    fn new() -> (Self, Task<Msg>) {
        let root = store::data_dir().unwrap_or_default();
        i18n::reload(&root);
        let monitor = Self::open_monitor(&root).ok();
        let loaded = monitor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("{}", tr("Could not open local settings.")))
            .and_then(|s| s.snapshot());
        let notice = loaded
            .as_ref()
            .err()
            .map(|e| format!("{e:#}"))
            .unwrap_or_default();
        let state = loaded.unwrap_or_default();
        let mail = state.mail.clone();
        (
            Self {
                monitor,
                focused: true,
                update_preferences: updates::Preferences::load(&root),
                update_status: UpdateStatus::Idle,
                update_progress: None,
                root,
                state,
                page: if std::env::args().any(|a| a == "--activity") {
                    Page::History
                } else {
                    Page::Dashboard
                },
                connection: ConnectionForm::default(),
                plan: PlanForm::default(),
                dbs: HashMap::new(),
                expanded: HashSet::new(),
                selected_database: None,
                tables: vec![],
                tables_loading: false,
                tables_error: String::new(),
                tables_request: 0,
                table_filter: String::new(),
                table_page: 0,
                notice,
                error: false,
                busy: false,
                delete: None,
                mail,
                mail_port: "587".into(),
                mail_password: String::new(),
            },
            Task::done(Msg::CheckUpdates(false)),
        )
    }
    fn open_monitor(root: &std::path::Path) -> anyhow::Result<Store> {
        let store = Store::open(root)?;
        store.db.busy_timeout(Duration::from_millis(100))?;
        Ok(store)
    }
    fn refresh(&mut self) {
        if self.monitor.is_none() {
            self.monitor = Self::open_monitor(&self.root).ok();
        }
        let loaded = self
            .monitor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("{}", tr("Could not open local settings.")))
            .and_then(|s| s.snapshot());
        match loaded {
            Ok(s) => self.state = s,
            Err(e) => self.result(Err(e)),
        }
    }
    fn result(&mut self, r: anyhow::Result<String>) {
        match r {
            Ok(s) => {
                self.notice = s;
                self.error = false
            }
            Err(e) => {
                self.notice = format!("{e:#}");
                self.error = true
            }
        }
    }
    fn alive(&self) -> bool {
        self.state
            .heartbeat
            .is_some_and(|t| (Utc::now() - t).num_seconds() < 30)
    }
    fn update(&mut self, msg: Msg) -> Task<Msg> {
        let previous_page = self.page;
        let task = self.update_message(msg);
        if self.page != previous_page {
            Task::batch([
                task,
                iced::widget::operation::snap_to(
                    "page-content",
                    iced::widget::operation::RelativeOffset::START,
                ),
            ])
        } else {
            task
        }
    }
    fn update_message(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::CheckUpdates(manual) => {
                if self.update_progress.is_some()
                    || matches!(self.update_status, UpdateStatus::Checking)
                    || (!manual && !self.update_preferences.due(Utc::now()))
                {
                    return Task::none();
                }
                self.update_preferences.last_attempt = Some(Utc::now());
                if manual {
                    self.update_preferences.dismissed_version = None;
                }
                // Update discovery must never prevent a backup if preference persistence fails.
                if let Err(e) = self.update_preferences.save(&self.root) {
                    self.result(Err(e));
                }
                self.update_status = UpdateStatus::Checking;
                return Task::perform(
                    async { updates::check().await.map_err(|e| e.to_string()) },
                    Msg::UpdatesChecked,
                );
            }
            Msg::UpdatesChecked(result) => {
                match result {
                    Ok(updates::CheckResult::Available(release)) => {
                        self.update_preferences.last_release = Some(release);
                        self.update_status = UpdateStatus::Idle;
                    }
                    Ok(updates::CheckResult::Current) => {
                        self.update_preferences.last_release = None;
                        self.update_status = UpdateStatus::Current;
                    }
                    Ok(updates::CheckResult::NotPublished) => {
                        self.update_status = UpdateStatus::NotPublished;
                    }
                    Err(_) => {
                        self.update_status = UpdateStatus::Failed;
                    }
                }
                if let Err(e) = self.update_preferences.save(&self.root) {
                    self.result(Err(e));
                }
            }
            Msg::AutomaticUpdates(enabled) => {
                let mut preferences = self.update_preferences.clone();
                preferences.automatic = enabled;
                match preferences.save(&self.root) {
                    Ok(()) => self.update_preferences = preferences,
                    Err(e) => self.result(Err(e)),
                }
            }
            Msg::DismissUpdate => {
                self.update_preferences.dismissed_version =
                    self.available_update().map(|r| r.version.clone());
                if let Err(e) = self.update_preferences.save(&self.root) {
                    self.result(Err(e));
                }
            }
            Msg::DownloadUpdate => {
                if self.update_progress.is_some()
                    || matches!(self.update_status, UpdateStatus::Checking)
                {
                    return Task::none();
                }
                if let Some(release) = self.available_update().cloned() {
                    let progress = Arc::new(updater::Progress::default());
                    self.update_progress = Some(progress.clone());
                    let root = self.root.clone();
                    return Task::perform(
                        async move {
                            updater::download(root, release, progress)
                                .await
                                .map_err(|e| format!("{e:#}"))
                        },
                        Msg::UpdateDownloaded,
                    );
                }
            }
            Msg::CancelDownload => {
                if let Some(progress) = &self.update_progress
                    && progress.stage.load(Ordering::Relaxed) < 3
                {
                    progress.cancel.store(true, Ordering::Relaxed);
                }
            }
            Msg::UpdateDownloaded(result) => match result {
                Ok(prepared)
                    if self
                        .update_progress
                        .as_ref()
                        .is_some_and(|p| !p.cancel.load(Ordering::Relaxed)) =>
                {
                    if let Some(progress) = &self.update_progress {
                        progress.stage.store(3, Ordering::Relaxed);
                    }
                    let root = self.root.clone();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || updater::handoff(&root, prepared))
                                .await
                                .map_err(|e| e.to_string())?
                                .map_err(|e| format!("{e:#}"))
                        },
                        Msg::UpdateHandoff,
                    );
                }
                Err(e) => {
                    let cancelled = self
                        .update_progress
                        .as_ref()
                        .is_some_and(|p| p.cancel.load(Ordering::Relaxed));
                    self.update_progress = None;
                    if cancelled {
                        self.result(Ok(tr("Update download cancelled.").into()));
                    } else {
                        self.result(Err(anyhow::anyhow!("{} {e}", tr("The update could not be completed. Your settings and backups are kept."))));
                    }
                }
                _ => {
                    self.update_progress = None;
                    self.result(Ok(tr("Update download cancelled.").into()));
                }
            },
            Msg::UpdateHandoff(result) => {
                match result {
                    Ok(()) => return iced::exit(),
                    Err(e) => {
                        self.update_progress = None;
                        self.result(Err(anyhow::anyhow!("{} {e}", tr("The update could not be completed. Your settings and backups are kept."))));
                    }
                }
            }
            Msg::Language(language) => {
                self.result(
                    i18n::save(&self.root, language).map(|_| tr("Language saved.").to_owned()),
                );
            }
            Msg::WindowEvent(iced::window::Event::Focused) => {
                self.focused = true;
            }
            Msg::WindowEvent(iced::window::Event::Unfocused) => {
                self.focused = false;
            }
            Msg::WindowEvent(_) => {}
            Msg::Animate => {}
            Msg::Tick => {
                self.refresh();
                if let Ok(Some(activity)) = self
                    .monitor
                    .as_ref()
                    .map(|s| s.take_window_request())
                    .transpose()
                    .map(Option::flatten)
                {
                    if activity {
                        self.page = Page::History;
                    }
                    return iced::window::latest().and_then(|id| {
                        Task::batch([
                            iced::window::minimize(id, false),
                            iced::window::gain_focus(id),
                        ])
                    });
                }
                if !matches!(self.update_status, UpdateStatus::Checking)
                    && self.update_preferences.due(Utc::now())
                {
                    return self.update_message(Msg::CheckUpdates(false));
                }
            }
            Msg::Page(p) => {
                self.page = p;
                self.delete = None;
                if p == Page::Settings {
                    self.mail = self.state.mail.clone();
                    self.mail_port = if self.mail.port == 0 {
                        "587".into()
                    } else {
                        self.mail.port.to_string()
                    };
                }
            }
            Msg::NewConnection => {
                self.connection.password.zeroize();
                self.connection = ConnectionForm::default();
                self.page = Page::ConnectionForm;
                self.notice.clear();
            }
            Msg::EditConnection(id) => {
                if let Some(c) = self.state.connections.iter().find(|c| c.id == id) {
                    self.connection.password.zeroize();
                    self.connection = ConnectionForm::from(c);
                    self.page = Page::ConnectionForm;
                }
            }
            Msg::NewPlan => {
                self.plan = PlanForm::default();
                self.plan.connection = self.state.connections.first().map(|c| Choice {
                    id: c.id.clone(),
                    name: c.name.clone(),
                });
                self.page = Page::PlanForm;
                self.notice.clear();
            }
            Msg::EditPlan(id) => {
                if let Some(p) = self.state.plans.iter().find(|p| p.id == id) {
                    self.plan = PlanForm::from(p, &self.state);
                    self.page = Page::PlanForm;
                }
            }
            Msg::Engine(e) => {
                self.connection.engine = e;
                self.connection.port = e.port().to_string();
            }
            Msg::Frequency(f) => self.plan.frequency = f,
            Msg::Compression(c) => self.plan.compression = c,
            Msg::DestinationProvider(provider) => {
                self.plan.remote = Remote {
                    provider,
                    ..Remote::default()
                };
                self.plan.remote_port = "21".into();
                self.plan.remote_secret.zeroize();
                self.plan.remote_saved = false;
            }
            Msg::DestinationTls(tls) => self.plan.remote.tls = tls,
            Msg::AuthorizeDestination => {
                if self.busy {
                    return Task::none();
                }
                self.plan.remote.id = uuid::Uuid::new_v4().to_string();
                self.plan.remote_secret.zeroize();
                self.plan.remote_saved = false;
                let id = self.plan.remote.id.clone();
                let provider = self.plan.remote.provider;
                self.busy = true;
                self.notice = tr(
                    "Complete account authorization in your browser. This may take a few minutes.",
                )
                .into();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            destinations::authorize(provider).map_err(|e| format!("{e:#}"))
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |result| Msg::DestinationAuthorized(id.clone(), result),
                );
            }
            Msg::DestinationAuthorized(id, result) => {
                self.busy = false;
                if self.plan.remote.id == id {
                    match result {
                        Ok(token) => {
                            self.plan.remote_secret = token.0;
                            self.notice =
                                tr("Account connected. Save the plan to keep this authorization.")
                                    .into();
                            self.error = false;
                        }
                        Err(e) => self.result(Err(anyhow::anyhow!(e))),
                    }
                }
            }
            Msg::TestDestination => {
                if self.busy {
                    return Task::none();
                }
                let setup = (|| -> anyhow::Result<_> {
                    let remote = self.plan.remote_model()?;
                    let secret = if self.plan.remote_secret.is_empty() {
                        Store::open(&self.root)?.secret(&remote.secret_key())?
                    } else {
                        self.plan.remote_secret.clone()
                    };
                    remote.validate_secret(&secret)?;
                    Ok((remote, secret))
                })();
                match setup {
                    Ok((remote, secret)) => {
                        self.busy = true;
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || destinations::test(&remote,&secret).map(|_|tr("Destination is reachable. Write access is verified during backup.").into()).map_err(|e|format!("{e:#}"))).await.unwrap_or_else(|e|Err(e.to_string()))
                            },
                            Msg::Done,
                        );
                    }
                    Err(e) => self.result(Err(e)),
                }
            }
            Msg::Tls(v) => self.connection.tls = v,
            Msg::Notify(v) => self.plan.notify = v,
            Msg::ImplicitTls(v) => {
                self.mail.implicit_tls = v;
                self.mail_port = if v { "465" } else { "587" }.into();
            }
            Msg::SelectConnection(c) => {
                self.plan.connection = Some(c);
                self.plan.databases.clear();
            }
            Msg::SelectDb(db, selected) => {
                self.plan.databases.retain(|d| d != &db);
                if selected {
                    self.plan.databases.push(db);
                }
            }
            Msg::Field(k, v) => match k {
                "cn" => self.connection.name = v,
                "host" => self.connection.host = v,
                "port" => self.connection.port = v,
                "user" => self.connection.user = v,
                "password" => self.connection.password = v,
                "tools" => self.connection.tools = v,
                "ca" => self.connection.ca = v,
                "pn" => self.plan.name = v,
                "dest" => self.plan.destination = v,
                "remote_host" => self.plan.remote.host = v,
                "remote_port" => self.plan.remote_port = v,
                "remote_user" => self.plan.remote.user = v,
                "remote_secret" => self.plan.remote_secret = Zeroizing::new(v),
                "remote_path" => self.plan.remote.path = v,
                "remote_region" => self.plan.remote.region = v,
                "remote_bucket" => self.plan.remote.bucket = v,
                "remote_bandwidth" => self.plan.remote_bandwidth = v,
                "dbs" => self.plan.manual_dbs = v,
                "table_filter" => {
                    self.table_filter = v;
                    self.table_page = 0;
                }
                "time" => self.plan.time = v,
                "minutes" => self.plan.minutes = v,
                "weekday" => self.plan.weekday = v,
                "day" => self.plan.day = v,
                "month" => self.plan.month = v,
                "date" => self.plan.date = v,
                "tz" => self.plan.timezone = v,
                "count" => self.plan.count = v,
                "days" => self.plan.days = v,
                "timeout" => self.plan.timeout = v,
                "smtp" => self.mail.host = v,
                "smtpport" => self.mail_port = v,
                "smtpuser" => self.mail.username = v,
                "smtppass" => self.mail_password = v,
                "from" => self.mail.from = v,
                "to" => self.mail.to = v,
                _ => {}
            },
            Msg::Browse(k) => {
                return Task::perform(
                    async move {
                        let dialog = rfd::AsyncFileDialog::new();
                        let p = if k == "ca" {
                            dialog.pick_file().await
                        } else {
                            dialog.pick_folder().await
                        };
                        p.map(|p| p.path().to_path_buf())
                    },
                    move |p| Msg::Browsed(k, p),
                );
            }
            Msg::Browsed(k, Some(p)) => {
                return self.update(Msg::Field(k, p.to_string_lossy().into()));
            }
            Msg::Browsed(_, None) => {}
            Msg::CustomTools(enabled) => self.connection.custom_tools = enabled,
            Msg::SaveConnection => {
                let r = (|| {
                    let c = self.connection.model()?;
                    Store::open(&self.root)?.save_connection(&c, &self.connection.password)?;
                    Ok(
                        tr("Connection saved. Credentials protected by the operating system.")
                            .into(),
                    )
                })();
                if r.is_ok() {
                    self.connection.password.zeroize();
                    self.page = Page::Connections;
                }
                self.result(r);
                self.refresh();
            }
            Msg::SavePlan => {
                let r = (|| {
                    let p = self.plan.model()?;
                    if p.notify {
                        mail::validate(&self.state.mail)?;
                    }
                    Store::open(&self.root)?.save_plan_destination(&p, &self.plan.remote_secret)?;
                    Ok(tr("Plan saved. Start the agent for automatic backups.").into())
                })();
                if r.is_ok() {
                    self.plan.remote_secret.zeroize();
                    self.page = Page::Plans;
                }
                self.result(r);
                self.refresh();
            }
            Msg::ToggleExplorer(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id.clone());
                    if !self.dbs.contains_key(&id) && !self.busy {
                        return self.update(Msg::Discover(id));
                    }
                }
            }
            Msg::TablePage(page) => self.table_page = page,
            Msg::SelectDatabase(id, database) => {
                self.page = Page::Tables;
                self.selected_database = Some((id.clone(), database.clone()));
                self.tables.clear();
                self.tables_error.clear();
                self.table_filter.clear();
                self.table_page = 0;
                self.tables_request += 1;
                let request = self.tables_request;
                self.tables_loading = true;
                let root = self.root.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let result = (|| {
                                let store = Store::open(&root)?;
                                let connection: Connection = store.get("connection", &id)?;
                                let password = store.secret(&id)?;
                                engine::tables(
                                    &connection,
                                    &password,
                                    &database,
                                    &root.join("temp"),
                                )
                            })();
                            result.map_err(|e: anyhow::Error| format!("{e:#}"))
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |result| Msg::TablesLoaded(request, result),
                );
            }
            Msg::TablesLoaded(request, result) => {
                if request == self.tables_request {
                    self.tables_loading = false;
                    match result {
                        Ok(tables) => self.tables = tables,
                        Err(error) => self.tables_error = error,
                    }
                }
            }
            Msg::Discover(id) => {
                self.expanded.insert(id.clone());
                let r = (|| {
                    let s = Store::open(&self.root)?;
                    let c: Connection = s.get("connection", &id)?;
                    let p = s.secret(&id)?;
                    Ok((c, p))
                })();
                match r {
                    Ok((c, p)) => {
                        let temp = self.root.join("temp");
                        self.busy = true;
                        self.notice = tr("Testing connection and reading databases…").into();
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    engine::databases(&c, &p, &temp).map_err(|e| format!("{e:#}"))
                                })
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()))
                            },
                            move |r| Msg::Discovered(id.clone(), r),
                        );
                    }
                    Err(e) => self.result(Err(e)),
                }
            }
            Msg::TestForm => {
                let r = (|| {
                    let c = self.connection.model()?;
                    c.validate()?;
                    let password = if self.connection.password.is_empty() {
                        Store::open(&self.root)?
                            .secret(&c.id)
                            .unwrap_or_else(|_| zeroize::Zeroizing::new(String::new()))
                    } else {
                        zeroize::Zeroizing::new(self.connection.password.clone())
                    };
                    Ok((c, password))
                })();
                match r {
                    Ok((c, p)) => {
                        let temp = self.root.join("temp");
                        let id = c.id.clone();
                        self.busy = true;
                        self.notice = tr("Testing connection…").into();
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    engine::databases(&c, &p, &temp).map_err(|e| format!("{e:#}"))
                                })
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()))
                            },
                            move |r| Msg::Discovered(id.clone(), r),
                        );
                    }
                    Err(e) => self.result(Err(e)),
                }
            }
            Msg::Discovered(id, r) => {
                self.busy = false;
                match r {
                    Ok(dbs) => {
                        self.notice = i18n::format(
                            "Connected. Databases available: {0}.",
                            &[(dbs.len()).to_string()],
                        );
                        self.error = false;
                        self.dbs.insert(id, dbs);
                    }
                    Err(e) => self.result(Err(anyhow::anyhow!(e))),
                }
            }
            Msg::Run(id) => {
                let r = (|| {
                    let s = Store::open(&self.root)?;
                    let p = s.get("plan", &id)?;
                    s.enqueue(&p)?;
                    worker::launch()?;
                    Ok(tr("Backup queued. Follow the result in Activity.").into())
                })();
                if r.is_ok() {
                    self.page = Page::History;
                }
                self.result(r);
                self.refresh();
            }
            Msg::Toggle(id) => {
                let r = (|| {
                    let s = Store::open(&self.root)?;
                    let mut p: Plan = s.get("plan", &id)?;
                    p.enabled = !p.enabled;
                    if p.enabled {
                        p.next_due = p.schedule.next_after(Utc::now());
                    }
                    s.save_plan(&p)?;
                    Ok(tr("Schedule updated.").into())
                })();
                self.result(r);
                self.refresh();
            }
            Msg::AskDelete(k, id) => self.delete = Some((k, id)),
            Msg::CancelDelete => self.delete = None,
            Msg::Delete => {
                if let Some((k, id)) = self.delete.take() {
                    let r = Store::open(&self.root)
                        .and_then(|s| s.delete(k, &id))
                        .map(|_| {
                            tr("Configuration removed. Backups and source databases kept.").into()
                        });
                    self.result(r);
                    self.refresh();
                }
            }
            Msg::LaunchAgent => {
                let r = worker::launch().map(|_| {
                    tr("Agent starting. It will keep running after the window closes.").into()
                });
                self.result(r);
            }
            Msg::StopAgent => {
                let r = Store::open(&self.root)
                    .and_then(|s| s.put("agent", "stop", &true))
                    .map(|_| {
                        tr("The agent will stop after the current operation finishes.").into()
                    });
                self.result(r);
            }
            Msg::SaveMail => {
                let r = (|| {
                    self.mail.port = self.mail_port.parse()?;
                    Store::open(&self.root)?.save_mail(&self.mail, &self.mail_password)?;
                    Ok(tr("SMTP settings saved. Enable alerts in the desired plans.").into())
                })();
                if r.is_ok() {
                    self.mail_password.zeroize();
                }
                self.result(r);
                self.refresh();
            }
            Msg::TestMail => {
                let root = self.root.clone();
                self.busy = true;
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let s = Store::open(&root)?;
                            let m = s.get("settings", "mail")?;
                            let p = s.secret("smtp")?;
                            mail::send(
                                &m,
                                &p,
                                tr("StrajaBackupSQL · test email"),
                                tr("Email notifications are configured correctly."),
                            )?;
                            Ok::<_, anyhow::Error>(tr("Test email sent.").to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| format!("{e:#}")))
                    },
                    Msg::Done,
                );
            }
            Msg::Verify(folder) => {
                self.busy = true;
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move|| { let _background = straja_sql::resources::BackgroundWork::enter(); engine::verify(&folder).map(|m|i18n::format("Files fully verified: {0}. SHA-256 and decompressed content match.", &[(m.files.len()).to_string()])).map_err(|e|format!("{e:#}")) }).await.unwrap_or_else(|e|Err(e.to_string()))
                    },
                    Msg::Done,
                );
            }
            Msg::OpenFolder(p) => {
                let r = straja_sql::platform::open(&p);
                if let Err(e) = r {
                    self.result(Err(e));
                }
            }
            Msg::Done(r) => {
                self.busy = false;
                self.result(r.map_err(anyhow::Error::msg));
            }
        }
        Task::none()
    }
    fn view(&self) -> Element<'_, Msg> {
        let nav = column![
            text("StrajaBackupSQL").size(20).color(Color::WHITE),
            text("SQL BACKUP STUDIO")
                .size(11)
                .color(Color::from_rgb8(111, 185, 172)),
            Space::new().height(30),
            nav_button(tr("01   Overview"), Page::Dashboard, self.page),
            nav_button(tr("02   Connections"), Page::Connections, self.page),
            nav_button(tr("03   Backup plans"), Page::Plans, self.page),
            nav_button(tr("04   Activity"), Page::History, self.page),
            Space::new().height(25),
            text(tr("DATABASE EXPLORER"))
                .size(10)
                .color(Color::from_rgb8(116, 138, 163)),
            self.explorer(),
            Space::new().height(Fill),
            nav_button(tr("Settings & notifications"), Page::Settings, self.page),
            nav_button(tr("User guide"), Page::Help, self.page),
            pick_list(Language::ALL, Some(i18n::current()), Msg::Language)
                .width(Fill)
                .padding(8),
            Space::new().height(10),
            text(concat!("WINDOWS  /  RUST  /  v", env!("CARGO_PKG_VERSION")))
                .size(10)
                .color(Color::from_rgb8(116, 138, 163))
        ]
        .spacing(10);
        let sidebar = container(nav)
            .padding(24)
            .width(280)
            .height(Fill)
            .style(|_| container::Style {
                background: Some(NAVY.into()),
                ..Default::default()
            });
        let (title, subtitle) = match self.page {
            Page::Dashboard => (
                tr("Protection, without the worry."),
                tr("One place for all your database backups."),
            ),
            Page::Connections => (
                tr("Your connections"),
                tr("MariaDB, MySQL and PostgreSQL. Organized in one place."),
            ),
            Page::Plans => (
                tr("Backup plans"),
                tr("Choose what to back up, when, and how long to keep it."),
            ),
            Page::Tables => (
                tr("Database tables"),
                tr("Select a database in the explorer to see its structure."),
            ),
            Page::History => (
                tr("Activity"),
                tr("Results, checks and errors in chronological order."),
            ),
            Page::Settings => (
                tr("Settings & notifications"),
                tr("Scheduling agent and email alerts."),
            ),
            Page::Help => (
                tr("User guide"),
                tr("From your first connection to a verified backup."),
            ),
            Page::ConnectionForm => (
                tr("Connection setup"),
                tr("Your credentials stay on this computer."),
            ),
            Page::PlanForm => (
                tr("Plan setup"),
                tr("A clear plan. Automatic backups. Control over retention."),
            ),
        };
        let head = row![
            column![text(title).size(29), text(subtitle).size(13).color(MUTED)]
                .spacing(7)
                .width(Fill),
            badge(
                if self.alive() {
                    tr("●  Agent online")
                } else {
                    tr("○  Agent stopped")
                },
                self.alive()
            )
        ]
        .align_y(iced::Alignment::Center);
        let body = match self.page {
            Page::Dashboard => self.dashboard(),
            Page::Connections => self.connections(),
            Page::Plans => self.plans(),
            Page::Tables => self.database_tables(),
            Page::History => self.history(),
            Page::Settings => self.settings(),
            Page::Help => self.help(),
            Page::ConnectionForm => self.connection_form(),
            Page::PlanForm => self.plan_form(),
        };
        let mut content = column![head].spacing(24);
        if self.available_update().is_some_and(|release| {
            self.update_preferences.dismissed_version.as_deref() != Some(&release.version)
        }) && self.page != Page::Settings
        {
            content = content.push(self.update_card(false));
        }
        if !self.notice.is_empty() {
            content = content.push(
                container(text(&self.notice).size(13).color(if self.error {
                    Color::from_rgb8(173, 45, 53)
                } else {
                    GREEN
                }))
                .padding(14)
                .width(Fill)
                .style(card_style),
            );
        }
        if let Some((kind, _)) = &self.delete {
            content = content.push(card(
                column![
                    text(if *kind == "plan" {
                        tr("Remove this plan and its schedule?")
                    } else {
                        tr("Remove this connection and its saved password?")
                    })
                    .size(17),
                    text(tr("Existing backups and databases on the server are kept.")).size(13),
                    row![
                        button(tr("Remove configuration"))
                            .on_press(Msg::Delete)
                            .style(button::danger),
                        button(tr("Cancel"))
                            .on_press(Msg::CancelDelete)
                            .style(button::secondary)
                    ]
                    .spacing(10)
                ]
                .spacing(12),
            ));
        }
        content = content.push(body);
        row![
            sidebar,
            scrollable(container(content).padding(32).width(Fill))
                .id("page-content")
                .width(Fill)
        ]
        .height(Fill)
        .into()
    }
    fn explorer(&self) -> Element<'_, Msg> {
        let mut content = column![].spacing(6);
        if self.state.connections.is_empty() {
            content = content.push(
                text(tr("Add your first connection to see databases."))
                    .size(12)
                    .color(MUTED),
            );
        }
        for connection in &self.state.connections {
            let expanded = self.expanded.contains(&connection.id);
            content = content.push(
                button(
                    text(format!(
                        "{}  {}",
                        if expanded { "⌄" } else { "›" },
                        connection.name
                    ))
                    .size(13)
                    .color(Color::from_rgb8(220, 230, 239)),
                )
                .width(Fill)
                .on_press(Msg::ToggleExplorer(connection.id.clone()))
                .style(button::text),
            );
            if expanded {
                if let Some(databases) = self.dbs.get(&connection.id) {
                    if databases.is_empty() {
                        content = content.push(
                            text(tr("     No databases available"))
                                .size(12)
                                .color(MUTED),
                        );
                    }
                    for database in databases {
                        let selected = self
                            .selected_database
                            .as_ref()
                            .is_some_and(|(id, db)| id == &connection.id && db == database);
                        content = content.push(
                            button(text(format!("    ◇  {database}")).size(12).color(
                                if selected {
                                    Color::WHITE
                                } else {
                                    Color::from_rgb8(144, 179, 186)
                                },
                            ))
                            .width(Fill)
                            .on_press(Msg::SelectDatabase(connection.id.clone(), database.clone()))
                            .style(if selected {
                                button::primary
                            } else {
                                button::text
                            }),
                        );
                    }
                }
                content = content.push(
                    button(if self.busy {
                        tr("Reading databases…")
                    } else {
                        tr("↻  Refresh databases")
                    })
                    .on_press_maybe((!self.busy).then(|| Msg::Discover(connection.id.clone())))
                    .style(button::text),
                );
            }
        }
        scrollable(content).height(Length::FillPortion(2)).into()
    }
    fn database_tables(&self) -> Element<'_, Msg> {
        let Some((id, database)) = &self.selected_database else {
            return empty(
                tr("Choose a database"),
                tr("Select a database in Database Explorer."),
            );
        };
        let connection = self.state.connections.iter().find(|c| &c.id == id);
        let title = format!(
            "{}  /  {}",
            connection
                .map(|c| c.name.as_str())
                .unwrap_or(tr("Connection")),
            database
        );
        let mut content = column![
            row![
                text(title).size(22).width(Fill),
                button(tr("Refresh"))
                    .on_press_maybe(
                        (!self.tables_loading)
                            .then(|| Msg::SelectDatabase(id.clone(), database.clone()))
                    )
                    .style(button::secondary)
            ]
            .spacing(12)
        ]
        .spacing(16);
        if self.tables_loading {
            content = content.push(text(tr("Reading tables…")).color(GREEN));
        } else if !self.tables_error.is_empty() {
            content = content.push(text(&self.tables_error).color(Color::from_rgb8(173, 45, 53)));
        } else {
            content = content.push(field(
                tr("Search table or schema"),
                tr("Table name…"),
                &self.table_filter,
                "table_filter",
            ));
            let query = self.table_filter.to_lowercase();
            let filtered = self
                .tables
                .iter()
                .filter(|t| {
                    t.name.to_lowercase().contains(&query)
                        || t.schema.to_lowercase().contains(&query)
                })
                .collect::<Vec<_>>();
            content = content.push(
                text(i18n::format(
                    "Objects: {0} · Matching filter: {1}",
                    &[
                        (self.tables.len()).to_string(),
                        (filtered.len()).to_string(),
                    ],
                ))
                .size(13)
                .color(MUTED),
            );
            if filtered.is_empty() {
                content = content.push(text(tr(
                    "No tables or views are visible for this selection and account.",
                )));
            } else {
                content = content.push(
                    row![
                        text(tr("SCHEMA")).width(Fill),
                        text(tr("NAME")).width(Length::FillPortion(2)),
                        text(tr("TYPE")).width(Fill)
                    ]
                    .spacing(14),
                );
                for table in filtered.iter().skip(self.table_page * 100).take(100) {
                    content = content.push(
                        row![
                            text(&table.schema).width(Fill),
                            text(&table.name).width(Length::FillPortion(2)),
                            text(i18n::label(&table.kind)).width(Fill)
                        ]
                        .spacing(14),
                    );
                }
                content = content.push(
                    row![
                        button(tr("Previous"))
                            .on_press_maybe(self.table_page.checked_sub(1).map(Msg::TablePage))
                            .style(button::secondary),
                        text(i18n::format(
                            "Page {0} / {1}",
                            &[
                                (self.table_page + 1).to_string(),
                                (filtered.len().div_ceil(100)).to_string()
                            ]
                        )),
                        button(tr("Next"))
                            .on_press_maybe(
                                ((self.table_page + 1) * 100 < filtered.len())
                                    .then_some(Msg::TablePage(self.table_page + 1))
                            )
                            .style(button::secondary)
                    ]
                    .spacing(14),
                );
            }
        }
        card(content)
    }
    fn dashboard(&self) -> Element<'_, Msg> {
        let successes = self
            .state
            .runs
            .iter()
            .filter(|r| r.status == "Reușit")
            .count();
        let failures = self
            .state
            .runs
            .iter()
            .filter(|r| r.status == "Eroare" || r.status == "Întrerupt")
            .count();
        let stats = row![
            stat(
                tr("CONNECTIONS"),
                self.state.connections.len().to_string(),
                tr("configured servers")
            ),
            stat(
                tr("ACTIVE PLANS"),
                self.state
                    .plans
                    .iter()
                    .filter(|p| p.enabled)
                    .count()
                    .to_string(),
                tr("schedules ready")
            ),
            stat(
                tr("SUCCESSFUL BACKUPS"),
                successes.to_string(),
                tr("in the last 200 runs")
            ),
            stat(
                tr("NEEDS ATTENTION"),
                failures.to_string(),
                tr("errors in recent history")
            )
        ]
        .spacing(14);
        let hero=container(row![column![text(tr("YOUR DATA.\nALWAYS BACKED UP.")).size(28).color(Color::WHITE),text(tr("Connect your server, choose databases and create\na plan to handle the rest.")).size(15).color(Color::from_rgb8(191,211,218)),row![button(tr("+  Create a plan")).padding([12,20]).on_press(Msg::NewPlan),button(tr("Add connection")).padding([12,20]).on_press(Msg::NewConnection).style(button::secondary)].spacing(12)].spacing(20).width(Fill),column![text("SQL").size(64).color(Color::from_rgb8(109,218,191)),text(tr("EXPORT  →  VERIFY\n          →  ARCHIVE")).size(12).color(Color::from_rgb8(191,211,218))].spacing(8)].spacing(24)).padding(30).width(Fill).style(|_|container::Style{background:Some(NAVY.into()),border:iced::Border{radius:14.0.into(),..Default::default()},..Default::default()});
        let mut next = column![text(tr("Coming up")).size(18)].spacing(14);
        let mut upcoming: Vec<_> = self
            .state
            .plans
            .iter()
            .filter(|p| p.enabled && p.next_due.is_some())
            .collect();
        upcoming.sort_by_key(|p| p.next_due);
        if upcoming.is_empty() {
            next = next.push(
                text(tr(
                    "No scheduled runs. Create your first plan to get started.",
                ))
                .size(13)
                .color(MUTED),
            );
        }
        for p in upcoming.into_iter().take(4) {
            next = next.push(
                row![
                    column![
                        text(&p.name).size(15),
                        text(p.schedule.label()).size(12).color(MUTED)
                    ]
                    .width(Fill),
                    text(
                        p.next_due
                            .map(|d| d
                                .with_timezone(&chrono::Local)
                                .format("%Y-%m-%d · %H:%M")
                                .to_string())
                            .unwrap_or_default()
                    )
                    .size(13)
                ]
                .spacing(12),
            );
        }
        column![
            stats,
            hero,
            card(next),
            row![
                badge(tr("Read-only SQL export"), true),
                badge(tr("SHA-256 integrity"), true),
                badge(tr("OS-protected credentials"), true)
            ]
            .spacing(14)
        ]
        .spacing(24)
        .into()
    }
    fn connections(&self) -> Element<'_, Msg> {
        let mut c = column![row![
            text(i18n::format(
                "Connections: {0}",
                &[(self.state.connections.len()).to_string()]
            ))
            .size(16)
            .width(Fill),
            button(tr("+  New connection"))
                .padding([10, 18])
                .on_press(Msg::NewConnection)
        ]]
        .spacing(18);
        if self.state.connections.is_empty() {
            c = c.push(empty(
                tr("Start with your first server"),
                tr("Add the server address and a dedicated backup account."),
            ));
        }
        for v in &self.state.connections {
            let mut body = column![
                row![
                    column![
                        text(&v.name).size(20),
                        text(format!("{} · {}:{} · {}", v.engine, v.host, v.port, v.user))
                            .size(13)
                            .color(MUTED)
                    ]
                    .spacing(6)
                    .width(Fill),
                    badge(
                        if v.tls {
                            tr("TLS verified")
                        } else {
                            tr("No TLS")
                        },
                        v.tls
                    )
                ],
                row![
                    button(tr("Test / Explore"))
                        .on_press_maybe((!self.busy).then(|| Msg::Discover(v.id.clone()))),
                    button(tr("Edit"))
                        .on_press(Msg::EditConnection(v.id.clone()))
                        .style(button::secondary),
                    button(tr("Remove…"))
                        .on_press(Msg::AskDelete("connection", v.id.clone()))
                        .style(button::text)
                ]
                .spacing(10)
            ]
            .spacing(18);
            if let Some(dbs) = self.dbs.get(&v.id) {
                body = body.push(text(dbs.join("   ·   ")).size(13).color(GREEN));
            }
            c = c.push(card(body));
        }
        c.into()
    }
    fn plans(&self) -> Element<'_, Msg> {
        let mut c = column![row![
            text(tr("Export, compression and retention per plan"))
                .size(14)
                .color(MUTED)
                .width(Fill),
            button(tr("+  New plan"))
                .padding([10, 18])
                .on_press(Msg::NewPlan)
        ]]
        .spacing(18);
        if self.state.plans.is_empty() {
            c = c.push(empty(
                tr("Your first backup starts with a plan"),
                tr("Select the connection, databases and folder for your backups."),
            ));
        }
        for p in &self.state.plans {
            let cn = self
                .state
                .connections
                .iter()
                .find(|c| c.id == p.connection)
                .map(|c| c.name.as_str())
                .unwrap_or(tr("Missing connection"));
            c = c.push(card(
                column![
                    row![
                        column![
                            text(&p.name).size(20),
                            text(format!("{cn}  /  {}", p.databases.join(", ")))
                                .size(13)
                                .color(MUTED)
                        ]
                        .spacing(6)
                        .width(Fill),
                        badge(
                            if p.enabled {
                                tr("Active")
                            } else {
                                tr("Paused")
                            },
                            p.enabled
                        )
                    ],
                    text(p.schedule.label()).size(14),
                    text(
                        p.remote
                            .as_ref()
                            .map(|r| r.label())
                            .unwrap_or_else(|| p.destination.display().to_string())
                    )
                    .size(12)
                    .color(MUTED),
                    text(i18n::format(
                        "{0}  ·  Retention: {1} copies / {2} days  ·  Email: {3}",
                        &[
                            (p.compression).to_string(),
                            p.keep_count
                                .map(|n| n.to_string())
                                .unwrap_or(tr("all").into()),
                            p.keep_days
                                .map(|n| n.to_string())
                                .unwrap_or(tr("unlimited").into()),
                            (if p.notify { tr("on") } else { tr("off") }).to_string()
                        ]
                    ))
                    .size(12),
                    row![
                        button(tr("Back up now")).on_press(Msg::Run(p.id.clone())),
                        button(tr("Edit"))
                            .on_press(Msg::EditPlan(p.id.clone()))
                            .style(button::secondary),
                        button(if p.enabled { tr("Pause") } else { tr("Enable") })
                            .on_press(Msg::Toggle(p.id.clone()))
                            .style(button::secondary),
                        button(tr("Remove…"))
                            .on_press(Msg::AskDelete("plan", p.id.clone()))
                            .style(button::text)
                    ]
                    .spacing(10)
                ]
                .spacing(15),
            ));
        }
        c.into()
    }
    fn history(&self) -> Element<'_, Msg> {
        let mut c = column![].spacing(14);
        if self.state.runs.is_empty() {
            c = c.push(empty(
                tr("Your history will appear here"),
                tr("After the first run, you will see the export and verification results."),
            ));
        }
        for r in &self.state.runs {
            let mut b = column![
                row![
                    text(&r.plan_name).size(18).width(Fill),
                    badge(i18n::label(&r.status), r.status == "Reușit")
                ],
                text(&r.started).size(12).color(MUTED),
                text(
                    if matches!(r.status.as_str(), "În lucru" | "În așteptare" | "Întrerupt") {
                        i18n::label(&r.detail)
                    } else {
                        &r.detail
                    }
                )
                .size(13)
            ]
            .spacing(12);
            if r.status == "În lucru" {
                let elapsed = chrono::DateTime::parse_from_rfc3339(&r.started)
                    .map(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds().max(0))
                    .unwrap_or(0);
                b = b.push(
                    text(i18n::format(
                        "Elapsed: {0} min {1} s",
                        &[(elapsed / 60).to_string(), format!("{:02}", elapsed % 60)],
                    ))
                    .size(13)
                    .color(MUTED),
                );
                if let Some(progress) = &r.progress {
                    b = b.push(
                        text(i18n::format(
                            "{0} · {1} · database {2} of {3}",
                            &[
                                (i18n::label(&progress.phase)).to_string(),
                                (progress.database).to_string(),
                                (progress.database_index).to_string(),
                                (progress.database_count).to_string(),
                            ],
                        ))
                        .size(16)
                        .color(GREEN),
                    );
                    if let Some(percent) = progress.percent() {
                        b = b.push(progress_bar(0.0..=100.0, percent).girth(8)).push(
                            text(i18n::format(
                                "{0}% of current stage · {1} / {2}",
                                &[
                                    format!("{:.0}", percent),
                                    (byte_size(progress.bytes)).to_string(),
                                    (byte_size(progress.total_bytes.unwrap_or(0))).to_string(),
                                ],
                            ))
                            .size(13),
                        );
                    } else {
                        if self.alive() {
                            let position =
                                (Utc::now().timestamp_millis().rem_euclid(4000) / 40) as u16 + 1;
                            b = b.push(
                                container(
                                    row![
                                        Space::new().width(Length::FillPortion(position)),
                                        container(Space::new()).width(70).height(8).style(|_| {
                                            container::Style::default().background(GREEN)
                                        }),
                                        Space::new().width(Length::FillPortion(101 - position))
                                    ]
                                    .width(Fill),
                                )
                                .width(Fill)
                                .style(|_| container::Style::default().background(LINE)),
                            );
                        }
                        b = b.push(
                            text(i18n::format(
                                "In progress · {0} processed",
                                &[(byte_size(progress.bytes)).to_string()],
                            ))
                            .size(14),
                        );
                        if progress.phase == "Export" {
                            b = b.push(text(tr("The SQL tool does not report a total export size. The processed size updates during the run.")).size(12).color(MUTED));
                        }
                    }
                    let stale = (Utc::now() - progress.updated).num_seconds();
                    if stale > 15 {
                        b = b.push(
                            text(i18n::format(
                                "Last updated {0} s ago",
                                &[(stale).to_string()],
                            ))
                            .size(12)
                            .color(MUTED),
                        );
                    }
                } else {
                    b = b.push(text(tr("Preparing export…")).color(GREEN));
                }
                if !self.alive() {
                    b = b.push(
                        text(tr(
                            "The agent is not responding; progress may be interrupted.",
                        ))
                        .color(Color::from_rgb8(173, 45, 53)),
                    );
                }
            } else if r.status == "Reușit" {
                b = b.push(progress_bar(0.0..=100.0, 100.0).girth(6)).push(
                    text(tr("100% · Backup completed and verified"))
                        .size(13)
                        .color(GREEN),
                );
            }
            if let Some(p) = &r.folder {
                b = b.push(
                    row![
                        button(tr("Open backup"))
                            .on_press(Msg::OpenFolder(p.clone()))
                            .style(button::secondary),
                        button(tr("Verify again"))
                            .on_press_maybe((!self.busy).then(|| Msg::Verify(p.clone())))
                            .style(button::secondary)
                    ]
                    .spacing(10),
                );
            }
            c = c.push(card(b));
        }
        c.into()
    }
    fn connection_form(&self) -> Element<'_, Msg> {
        let f = &self.connection;
        let mut tools = column![
            text(i18n::format("{0} {1} · bundled tools", &[(f.engine).to_string(), (f.engine.bundled_version()).to_string()])).size(15).color(GREEN),
            text(if f.custom_tools { tr("The custom folder below is active. Clear this option to use bundled tools.") } else { tr("Selected automatically from StrajaBackupSQL. Existing SQL installations remain independent.") }).size(12).color(MUTED),
            checkbox(f.custom_tools).label(tr("Advanced: use a different tools folder")).on_toggle(Msg::CustomTools),
        ].spacing(10);
        if f.custom_tools {
            tools = tools.push(path_field(tr("Custom bin folder"), &f.tools, "tools"))
                .push(text(i18n::format("Required: {0} + {1} and their libraries. Choose a version compatible with the server.", &[(f.engine.client()).to_string(), (f.engine.dump()).to_string()])).size(12).color(MUTED));
        }

        card(column![text(tr("01   Connection details")).size(18),field(tr("Connection name"),tr("Production · shop"),&f.name,"cn"),row![column![label(tr("SQL engine")),pick_list(Engine::ALL,Some(f.engine),Msg::Engine).width(Fill).padding(10)].spacing(7).width(Fill),field(tr("Port"),"3306",&f.port,"port")].spacing(20),field(tr("Server / IP address"),"db.example.com",&f.host,"host"),row![field(tr("Username"),"backup_reader",&f.user,"user"),secret(tr("Password · blank keeps the saved password"),&f.password,"password")].spacing(20),text(tr("02   Tools and security")).size(18),tools,checkbox(f.tls).label(tr("TLS with certificate and server identity verification")).on_toggle(Msg::Tls),path_field(tr("CA certificate (optional if already trusted)"),&f.ca,"ca"),text(tr("Use an account with read and export permissions. This app has no SQL editor, server restore or database deletion commands.")).size(12).color(MUTED),row![button(tr("Save connection")).padding([11,18]).on_press(Msg::SaveConnection),button(if self.busy{tr("Testing…")}else{tr("Test connection")}).padding([11,18]).on_press_maybe((!self.busy).then_some(Msg::TestForm)).style(button::secondary),button(tr("Back")).on_press(Msg::Page(Page::Connections)).style(button::text)].spacing(12)].spacing(20))
    }
    fn plan_form(&self) -> Element<'_, Msg> {
        let f = &self.plan;
        let choices: Vec<_> = self
            .state
            .connections
            .iter()
            .map(|c| Choice {
                id: c.id.clone(),
                name: c.name.clone(),
            })
            .collect();
        let mut databases = column![].spacing(8);
        if let Some(choice) = &f.connection {
            let mut names = self.dbs.get(&choice.id).cloned().unwrap_or_default();
            for db in &f.databases {
                if !names.contains(db) {
                    names.push(db.clone());
                }
            }
            for db in names {
                let checked = f.databases.contains(&db);
                let name = db.clone();
                databases = databases.push(
                    checkbox(checked)
                        .label(db)
                        .on_toggle(move |v| Msg::SelectDb(name.clone(), v)),
                );
            }
        }
        let mut schedule = column![
            row![
                column![
                    label(tr("Frequency")),
                    pick_list(Frequency::ALL, Some(f.frequency), Msg::Frequency)
                        .padding(10)
                        .width(Fill)
                ]
                .spacing(7)
                .width(Fill),
                field(tr("Time (HH:MM)"), "02:00", &f.time, "time")
            ]
            .spacing(20)
        ]
        .spacing(14);
        match f.frequency {
            Frequency::Minutes => {
                schedule = schedule.push(field(
                    tr("Interval in minutes"),
                    "60",
                    &f.minutes,
                    "minutes",
                ))
            }
            Frequency::Weekly => {
                schedule = schedule.push(field(
                    tr("Day · 1=Monday, 7=Sunday"),
                    "1",
                    &f.weekday,
                    "weekday",
                ))
            }
            Frequency::Monthly => {
                schedule = schedule.push(field(
                    tr("Day of month · 29–31 adjust to the last day"),
                    "1",
                    &f.day,
                    "day",
                ))
            }
            Frequency::Yearly => {
                schedule = schedule.push(
                    row![
                        field(tr("Month (1–12)"), "1", &f.month, "month"),
                        field(tr("Day (1–31)"), "1", &f.day, "day")
                    ]
                    .spacing(20),
                )
            }
            Frequency::Once => {
                schedule = schedule.push(field(
                    tr("Date (YYYY-MM-DD)"),
                    "2026-12-01",
                    &f.date,
                    "date",
                ))
            }
            _ => {}
        }
        schedule = schedule.push(field(
            tr("IANA time zone"),
            "UTC / Europe/Berlin",
            &f.timezone,
            "tz",
        ));
        card(column![text(tr("01   What to protect")).size(18),field(tr("Plan name"),tr("Daily backup · production"),&f.name,"pn"),row![column![label(tr("Connection")),pick_list(choices,f.connection.clone(),Msg::SelectConnection).placeholder(tr("Choose connection")).width(Fill).padding(10)].spacing(7).width(Fill),button(tr("Read databases")).on_press_maybe(if self.busy{None}else{f.connection.as_ref().map(|c|Msg::Discover(c.id.clone()))}).style(button::secondary)].spacing(15).align_y(iced::Alignment::End),databases,field(tr("Or add exact names separated by ;"),tr("shop; billing"),&f.manual_dbs,"dbs"),text(tr("02   Where and when")).size(18),self.destination_form(),schedule,text(tr("New plans use UTC. Choose the IANA time zone for your schedule, for example Europe/Berlin or America/New_York.")).size(12).color(MUTED),text(tr("03   Compression and retention")).size(18),pick_list([Compression::Gzip,Compression::None],Some(f.compression),Msg::Compression).padding(10),row![field(tr("Keep at most N copies · blank = all"),tr("Unlimited"),&f.count,"count"),field(tr("Keep at most N days · blank = all"),tr("Unlimited"),&f.days,"days")].spacing(20),text(tr("Retention runs only after a verified backup. The newest copy is always kept. If both limits are set, copies exceeding either limit expire.")).size(12).color(MUTED),field(tr("Maximum export duration, in minutes"),"120",&f.timeout,"timeout"),checkbox(f.notify).label(tr("Email on backup failure (configure SMTP in Settings)")).on_toggle(Msg::Notify),row![button(tr("Save plan")).padding([11,18]).on_press(Msg::SavePlan),button(tr("Back")).on_press(Msg::Page(Page::Plans)).style(button::text)].spacing(15)].spacing(20))
    }
    fn destination_form(&self) -> Element<'_, Msg> {
        let f = &self.plan;
        let mut c = column![
            label(tr("Backup destination")),
            pick_list(
                Provider::ALL,
                Some(f.remote.provider),
                Msg::DestinationProvider
            )
            .padding(10)
            .width(Fill)
        ]
        .spacing(14);
        c = c.push(path_field(
            if f.remote.provider == Provider::Local {
                tr("Destination folder (local or UNC)")
            } else {
                tr("Local backup folder before upload")
            },
            &f.destination,
            "dest",
        ));
        if f.remote.provider == Provider::Local {
            return c.into();
        }
        c=c.push(text(tr("A verified local copy is kept. Remote verification reads the uploaded files back and may incur download charges.")).size(12).color(MUTED));
        match f.remote.provider {
            Provider::Ftp => {
                c = c
                    .push(
                        row![
                            field(
                                tr("FTP server"),
                                "ftp.example.com",
                                &f.remote.host,
                                "remote_host"
                            ),
                            field(tr("Port"), "21", &f.remote_port, "remote_port")
                        ]
                        .spacing(16),
                    )
                    .push(
                        row![
                            field(tr("Username"), "backup", &f.remote.user, "remote_user"),
                            secret(
                                tr("Password · blank keeps the saved password"),
                                &f.remote_secret,
                                "remote_secret"
                            )
                        ]
                        .spacing(16),
                    )
                    .push(
                        checkbox(f.remote.tls)
                            .label(tr("Require explicit FTPS (TLS, verified certificate)"))
                            .on_toggle(Msg::DestinationTls),
                    );
                if !f.remote.tls {
                    c = c.push(
                        text(tr(
                            "Plain FTP sends credentials and backup data without encryption.",
                        ))
                        .size(12)
                        .color(MUTED),
                    );
                }
            }
            Provider::S3 => {
                c = c
                    .push(
                        row![
                            field(
                                tr("S3 bucket"),
                                "company-backups",
                                &f.remote.bucket,
                                "remote_bucket"
                            ),
                            field(
                                tr("AWS region"),
                                "eu-central-1",
                                &f.remote.region,
                                "remote_region"
                            )
                        ]
                        .spacing(16),
                    )
                    .push(field(
                        tr("Access key ID"),
                        "AKIA...",
                        &f.remote.user,
                        "remote_user",
                    ))
                    .push(secret(
                        tr("Secret access key · blank keeps the saved key"),
                        &f.remote_secret,
                        "remote_secret",
                    ));
            }
            Provider::GoogleDrive | Provider::Dropbox => {
                c = c
                    .push(
                        text(if !f.remote_secret.is_empty() {
                            tr("Account connected. Save the plan to keep this authorization.")
                        } else if f.remote_saved {
                            tr("Saved account authorization")
                        } else {
                            tr("Connect your account to enable scheduled uploads.")
                        })
                        .size(13),
                    )
                    .push(
                        button(tr("Connect account in browser"))
                            .on_press_maybe((!self.busy).then_some(Msg::AuthorizeDestination)),
                    );
            }
            Provider::Local => {}
        }
        c = c
            .push(field(
                tr("Remote folder / prefix"),
                "Backups",
                &f.remote.path,
                "remote_path",
            ))
            .push(field(
                tr("Transfer limit (MiB/s) · 0 = unlimited"),
                "0",
                &f.remote_bandwidth,
                "remote_bandwidth",
            ))
            .push(
                button(tr("Test destination"))
                    .on_press_maybe((!self.busy).then_some(Msg::TestDestination))
                    .style(button::secondary),
            );
        c.into()
    }
    fn available_update(&self) -> Option<&updates::Release> {
        self.update_preferences
            .last_release
            .as_ref()
            .filter(|release| {
                release
                    .is_newer_than(updates::CURRENT_VERSION)
                    .unwrap_or(false)
            })
    }
    fn update_card(&self, settings: bool) -> Element<'_, Msg> {
        let mut content = column![text(tr("Software updates")).size(20)].spacing(12);
        if settings {
            content = content
                .push(
                    text(i18n::format(
                        "Installed version: {0}",
                        &[updates::CURRENT_VERSION.into()],
                    ))
                    .size(13),
                )
                .push(
                    checkbox(self.update_preferences.automatic)
                        .label(tr(
                            "Check automatically every 24 hours while the app is open",
                        ))
                        .on_toggle(Msg::AutomaticUpdates),
                );
            let status = match self.update_status {
                UpdateStatus::Idle => {
                    tr("Updates are provided through the official StrajaBackupSQL GitHub releases.")
                }
                UpdateStatus::Checking => tr("Checking for updates…"),
                UpdateStatus::Current => {
                    tr("You have the latest version available on this channel.")
                }
                UpdateStatus::NotPublished => tr("The update feed has not been published yet."),
                UpdateStatus::Failed => tr("Could not check for updates. Please try again later."),
            };
            content = content.push(text(status).size(13).color(MUTED));
            if let Some(at) = self.update_preferences.last_attempt {
                content = content.push(
                    text(i18n::format(
                        "Last attempt (UTC): {0}",
                        &[at.format("%Y-%m-%d %H:%M").to_string()],
                    ))
                    .size(12)
                    .color(MUTED),
                );
            }
            content = content.push(
                button(tr("Check for updates")).on_press_maybe(
                    (self.update_progress.is_none()
                        && !matches!(self.update_status, UpdateStatus::Checking))
                    .then_some(Msg::CheckUpdates(true)),
                ),
            );
        }
        if let Some(release) = self.available_update() {
            content = content.push(text(i18n::format("StrajaBackupSQL {0} is available.", std::slice::from_ref(&release.version))).size(16).color(GREEN))
                .push(text(if cfg!(windows) { tr("Download and install here. The current backup finishes first, then the app restarts automatically. Your settings and backups are kept.") } else { tr("Download the verified package here. The current backup finishes first. Complete installation in the system installer, then reopen the app. Settings and backups are kept.") }).size(13).color(MUTED));
            if let Some(progress) = &self.update_progress {
                let stage = progress.stage.load(Ordering::Relaxed);
                let description = match stage {
                    0 => tr("Preparing update download…"),
                    1 => tr("Downloading update…"),
                    2 => tr("Verifying installer…"),
                    _ => tr(
                        "Waiting for the backup agent. The app will restart to install the update.",
                    ),
                };
                content = content.push(text(description).size(14).color(GREEN));
                let bytes = progress.bytes.load(Ordering::Relaxed);
                let total = progress.total.load(Ordering::Relaxed);
                if total > 0 {
                    let percent = (bytes as f64 / total as f64 * 100.).min(100.) as f32;
                    content = content.push(progress_bar(0.0..=100.0, percent)).push(
                        text(format!(
                            "{percent:.0}% · {:.1} / {:.1} MiB",
                            bytes as f64 / 1048576.,
                            total as f64 / 1048576.
                        ))
                        .size(12),
                    );
                }
                if stage < 3 {
                    content = content.push(
                        button(tr("Cancel download"))
                            .on_press(Msg::CancelDownload)
                            .style(button::secondary),
                    );
                }
                return card(content);
            }
            let mut actions = row![
                button(if cfg!(windows) {
                    tr("Download and install")
                } else {
                    tr("Download installer")
                })
                .on_press_maybe(
                    (!matches!(self.update_status, UpdateStatus::Checking))
                        .then_some(Msg::DownloadUpdate)
                )
            ]
            .spacing(12);
            if !settings {
                actions = actions.push(
                    button(tr("Skip this version"))
                        .on_press(Msg::DismissUpdate)
                        .style(button::secondary),
                );
            }
            content = content.push(actions);
        }
        card(content)
    }
    fn settings(&self) -> Element<'_, Msg> {
        column![self.update_card(true),card(column![text(tr("Language")).size(20),pick_list(Language::ALL,Some(i18n::current()),Msg::Language).padding(10),text(tr("Choose your language. Menus and new notifications update automatically. Existing log messages keep their original language.")).size(13).color(MUTED)].spacing(16)),card(column![text("StrajaBackupSQL Agent").size(20),text(if cfg!(windows) { tr("The agent runs independently of the window and appears beside the clock (also check ^). Right-click its icon to open the app, view Activity or stop after backup. Setup can enable startup at sign-in. The PC must stay on; the agent does not wake it from sleep.") } else { tr("The agent runs independently after you open the app and at login after package installation. Manage it here. A tray icon is not included in this platform preview. The computer must remain awake and your credential store unlocked.") }).size(13).color(MUTED),row![button(tr("Start agent")).on_press(Msg::LaunchAgent),button(tr("Stop after current run")).on_press(Msg::StopAgent).style(button::secondary)].spacing(12)].spacing(16)),card(column![text(tr("Email alerts")).size(20),text(tr("Sent only for plans with notifications enabled. SMTP connections require TLS.")).size(13).color(MUTED),row![field(tr("SMTP server"),"smtp.example.com",&self.mail.host,"smtp"),field(tr("Port"),"587",&self.mail_port,"smtpport")].spacing(20),row![field(tr("SMTP username"),"backup@example.com",&self.mail.username,"smtpuser"),secret(tr("SMTP password / app password"),&self.mail_password,"smtppass")].spacing(20),row![field(tr("Sender"),"backup@example.com",&self.mail.from,"from"),field(tr("Recipient"),"admin@example.com",&self.mail.to,"to")].spacing(20),checkbox(self.mail.implicit_tls).label(tr("Implicit TLS (usually 465); unchecked = STARTTLS (usually 587)")).on_toggle(Msg::ImplicitTls),row![button(tr("Save SMTP")).on_press(Msg::SaveMail),button(tr("Send test using saved settings")).on_press_maybe((!self.busy).then_some(Msg::TestMail)).style(button::secondary)].spacing(12)].spacing(18))].spacing(22).into()
    }
    fn help(&self) -> Element<'_, Msg> {
        card(column![text(tr("Get started in four steps")).size(22),text(tr("1. MariaDB, MySQL and PostgreSQL tools are bundled and selected automatically. For another version, enable a custom tools folder in advanced connection options.")).size(15),text(tr("2. Add and test the connection. Use a dedicated export account with minimal privileges. System databases are hidden in the explorer.")).size(15),text(tr("3. Create a plan: select databases, destination, compression and schedule. Run a manual backup and check the result in Activity.")).size(15),text(tr("4. Start the agent. Configure SMTP and enable alerts in each plan. Keep an additional copy on another drive or server.")).size(15),text(tr("What we verify")).size(20),text(tr("We check the tool exit code, export completion marker, SHA-256 reread and decompressed GZIP content. These checks detect file corruption but do not prove a full restore will succeed. Regularly test restoration on a separate server with official tools.")).size(14).color(MUTED),text(tr("Consistency and scheduling")).size(20),text(tr("MySQL/MariaDB use single-transaction: consistency applies to transactional tables (InnoDB). Avoid schema changes during export. Databases are exported sequentially without a shared snapshot. PostgreSQL uses the pg_dump snapshot. Global PostgreSQL roles are not exported.")).size(14).color(MUTED),text(tr("When the agent returns, it catches up one overdue run per plan. Failures are not retried immediately; the next schedule or Back up now starts a new attempt. Nonexistent DST times are skipped; repeated times run once. Hourly means 60 minutes after saving or the scheduled run.")).size(14).color(MUTED),text(tr("Local data and uninstall")).size(20),text(if cfg!(windows) { tr("Settings are in %LOCALAPPDATA%\\StrajaSQL. Passwords use DPAPI and work only under the same Windows account. SQL/GZIP backups are not encrypted. Uninstall keeps settings and backups. Bundled tools are private to this app; they do not change PATH, services or existing SQL installations.") } else { tr("Settings are stored in your user profile. Credentials use the system keychain. SQL/GZIP backups are not encrypted. Removing the app keeps settings and backups. Bundled clients do not modify existing SQL servers.") }).size(14).color(MUTED)].spacing(20))
    }
}
fn label(s: &str) -> Element<'_, Msg> {
    text(s).size(12).color(MUTED).into()
}
fn field<'a>(
    title: &'a str,
    placeholder: &'a str,
    value: &'a str,
    key: &'static str,
) -> Element<'a, Msg> {
    column![
        label(title),
        text_input(placeholder, value)
            .on_input(move |s| Msg::Field(key, s))
            .padding(11)
            .size(14)
    ]
    .spacing(7)
    .width(Fill)
    .into()
}
fn secret<'a>(title: &'a str, value: &'a str, key: &'static str) -> Element<'a, Msg> {
    column![
        label(title),
        text_input("••••••••", value)
            .secure(true)
            .on_input(move |s| Msg::Field(key, s))
            .padding(11)
            .size(14)
    ]
    .spacing(7)
    .width(Fill)
    .into()
}
fn path_field<'a>(title: &'a str, value: &'a str, key: &'static str) -> Element<'a, Msg> {
    row![
        field(title, tr("Choose a location…"), value, key),
        button(tr("Browse…"))
            .padding(11)
            .on_press(Msg::Browse(key))
            .style(button::secondary)
    ]
    .spacing(10)
    .align_y(iced::Alignment::End)
    .into()
}
fn card_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::WHITE.into()),
        border: iced::Border {
            color: LINE,
            width: 1.,
            radius: 12.0.into(),
        },
        ..Default::default()
    }
}
fn card<'a>(content: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    container(content)
        .padding(24)
        .width(Fill)
        .style(card_style)
        .into()
}
fn empty<'a>(title: &'a str, subtitle: &'a str) -> Element<'a, Msg> {
    card(column![text(title).size(22), text(subtitle).size(14).color(MUTED)].spacing(12))
}
fn stat<'a>(title: &'a str, value: String, caption: &'a str) -> Element<'a, Msg> {
    card(
        column![
            text(title).size(10).color(MUTED),
            text(value).size(32),
            text(caption).size(11).color(MUTED)
        ]
        .spacing(10),
    )
}
fn badge<'a>(value: &'a str, good: bool) -> Element<'a, Msg> {
    container(text(value).size(12).color(if good { GREEN } else { MUTED }))
        .padding([7, 11])
        .style(move |_| container::Style {
            background: Some(
                if good {
                    Color::from_rgb8(228, 245, 239)
                } else {
                    BG
                }
                .into(),
            ),
            border: iced::Border {
                radius: 6.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}
fn nav_button(title: &str, page: Page, current: Page) -> Element<'_, Msg> {
    button(text(title).size(13))
        .width(Fill)
        .padding([12, 10])
        .on_press(Msg::Page(page))
        .style(move |_, status| button::Style {
            background: Some(
                if page == current {
                    Color::from_rgb8(37, 67, 79)
                } else if status == button::Status::Hovered {
                    Color::from_rgb8(30, 48, 69)
                } else {
                    NAVY
                }
                .into(),
            ),
            text_color: if page == current {
                Color::from_rgb8(139, 235, 204)
            } else {
                Color::from_rgb8(186, 201, 219)
            },
            border: iced::Border {
                radius: 7.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn byte_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GiB", bytes as f64 / (1024. * 1024. * 1024.))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024. * 1024.))
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.)
    }
}

#[cfg(test)]
mod explorer_tests {
    use super::*;
    fn app() -> App {
        App {
            root: PathBuf::new(),
            monitor: None,
            focused: true,
            update_preferences: updates::Preferences::default(),
            update_status: UpdateStatus::Idle,
            update_progress: None,
            state: Snapshot::default(),
            page: Page::Dashboard,
            connection: ConnectionForm::default(),
            plan: PlanForm::default(),
            dbs: HashMap::new(),
            expanded: HashSet::new(),
            selected_database: None,
            tables: vec![],
            tables_loading: false,
            tables_error: String::new(),
            tables_request: 0,
            table_filter: String::new(),
            table_page: 0,
            notice: String::new(),
            error: false,
            busy: false,
            delete: None,
            mail: MailSettings::default(),
            mail_port: "587".into(),
            mail_password: String::new(),
        }
    }
    #[test]
    fn language_selection_persists_and_all_pages_build() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = app();
        app.root = temp.path().to_path_buf();
        app.connection.name = "Eroare {0} 東京".into();
        for language in Language::ALL {
            let _ = app.update(Msg::Language(language));
            assert!(!app.error);
            assert_eq!(i18n::load(temp.path()), language);
            assert_eq!(app.connection.name, "Eroare {0} 東京");
            for page in [
                Page::Dashboard,
                Page::Connections,
                Page::Plans,
                Page::History,
                Page::Tables,
                Page::Settings,
                Page::Help,
                Page::ConnectionForm,
                Page::PlanForm,
            ] {
                app.page = page;
                let _view = app.view();
            }
            for provider in Provider::ALL {
                app.plan.remote.provider = provider;
                app.page = Page::PlanForm;
                let _view = app.view();
            }
        }
        i18n::set(Language::English);
    }
    #[test]
    fn switching_destinations_discards_credentials_and_late_authorization() {
        let mut app = app();
        let _ = app.update(Msg::DestinationProvider(Provider::GoogleDrive));
        let previous = app.plan.remote.id.clone();
        app.plan.remote_secret = Zeroizing::new("old-secret".into());
        let _ = app.update(Msg::DestinationProvider(Provider::S3));
        assert!(app.plan.remote_secret.is_empty());
        let _ = app.update(Msg::DestinationAuthorized(
            previous,
            Ok(Authorized(Zeroizing::new("late-token".into()))),
        ));
        assert!(app.plan.remote_secret.is_empty());
        assert_eq!(app.plan.remote.provider, Provider::S3);
    }
    #[test]
    fn update_download_progress_cancellation_and_failure_keep_the_window_usable() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app();
        app.root = root.path().into();
        app.update_preferences.last_release = Some(updates::Release {
            schema_version: 1,
            product: "strajabackupsql".into(),
            platform: straja_sql::platform::ID.into(),
            channel: "stable".into(),
            version: "99.0.0".into(),
            release_url: "https://github.com/weblikero/straja-backup-sql/releases/tag/v99.0.0"
                .into(),
        });
        let progress = Arc::new(updater::Progress::default());
        app.update_progress = Some(progress.clone());
        progress.total.store(100, Ordering::Relaxed);
        progress.bytes.store(50, Ordering::Relaxed);
        for stage in 0..4 {
            progress.stage.store(stage, Ordering::Relaxed);
            for language in Language::ALL {
                i18n::set(language);
                let _ = app.update_card(true);
                let _ = app.update_card(false);
            }
        }
        let _ = app.update(Msg::CheckUpdates(true));
        assert!(!matches!(app.update_status, UpdateStatus::Checking));
        let _ = app.update(Msg::CancelDownload);
        assert!(!progress.cancel.load(Ordering::Relaxed)); // Installer handoff cannot be cancelled mid-copy.
        progress.stage.store(1, Ordering::Relaxed);
        let _ = app.update(Msg::CancelDownload);
        assert!(progress.cancel.load(Ordering::Relaxed));
        let _ = app.update(Msg::UpdateDownloaded(Err("cancelled".into())));
        assert!(app.update_progress.is_none());
        assert!(!app.error);
        app.update_progress = Some(Arc::new(updater::Progress::default()));
        let _ = app.update(Msg::UpdateHandoff(Err("fixture failure".into())));
        assert!(app.update_progress.is_none() && app.error);
        assert!(app.available_update().is_some()); // Retry remains available.
        i18n::set(Language::English);
    }
    #[test]
    fn update_notice_can_be_dismissed_and_manual_check_overrides_automatic_preference() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app();
        app.root = root.path().into();
        let release = updates::Release {
            schema_version: 1,
            product: "strajabackupsql".into(),
            platform: straja_sql::platform::ID.into(),
            channel: "stable".into(),
            version: "99.0.0".into(),
            release_url: "https://github.com/weblikero/straja-backup-sql/releases/tag/v99.0.0"
                .into(),
        };
        let _ = app.update(Msg::UpdatesChecked(Ok(updates::CheckResult::Available(
            release,
        ))));
        assert!(app.available_update().is_some());
        let _ = app.update(Msg::DismissUpdate);
        assert_eq!(
            updates::Preferences::load(root.path())
                .dismissed_version
                .as_deref(),
            Some("99.0.0")
        );
        let _ = app.update(Msg::AutomaticUpdates(false));
        let _ = app.update(Msg::CheckUpdates(false));
        assert!(!matches!(app.update_status, UpdateStatus::Checking));
        // Tasks are intentionally not polled: these state tests never contact the network.
        let _ = app.update(Msg::CheckUpdates(true));
        assert!(matches!(app.update_status, UpdateStatus::Checking));
        assert!(app.update_preferences.dismissed_version.is_none());
        let _ = app.update(Msg::UpdatesChecked(Err("offline".into())));
        assert!(matches!(app.update_status, UpdateStatus::Failed));
        assert!(!app.busy && !app.error && app.state.runs.is_empty());
        assert!(app.available_update().is_some());
        let _view = app.update_card(true);
    }
    #[test]
    fn cached_connection_toggles_and_late_discovery_does_not_reopen_it() {
        let mut app = app();
        app.dbs.insert("connection".into(), vec!["shop".into()]);
        let _ = app.update(Msg::ToggleExplorer("connection".into()));
        assert!(app.expanded.contains("connection"));
        let _ = app.update(Msg::ToggleExplorer("connection".into()));
        assert!(!app.expanded.contains("connection"));
        let _ = app.update(Msg::Discovered(
            "connection".into(),
            Ok(vec!["shop".into()]),
        ));
        assert!(!app.expanded.contains("connection"));
    }
    #[test]
    fn selection_opens_tables_and_ignores_an_outdated_response() {
        let mut app = app();
        // Tasks are not polled: no local state or network access.
        let _ = app.update(Msg::SelectDatabase("connection".into(), "first".into()));
        let first = app.tables_request;
        let _ = app.update(Msg::SelectDatabase("connection".into(), "second".into()));
        let second = app.tables_request;
        assert_eq!(app.page, Page::Tables);
        assert_eq!(app.selected_database.as_ref().unwrap().1, "second");
        let _ = app.update(Msg::TablesLoaded(first, Err("old failure".into())));
        assert!(app.tables_loading && app.tables_error.is_empty());
        let _ = app.update(Msg::TablesLoaded(
            second,
            Ok(vec![TableInfo {
                schema: "public".into(),
                name: "orders".into(),
                kind: "Tabel".into(),
            }]),
        ));
        assert!(!app.tables_loading && app.tables[0].name == "orders");
        app.table_page = 2;
        let _ = app.update(Msg::Field("table_filter", "orders".into()));
        assert_eq!(app.table_page, 0);
        let _view = app.database_tables();
    }
}
