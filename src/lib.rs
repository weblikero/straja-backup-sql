pub mod destinations;
pub mod engine;
pub mod i18n;
pub mod mail;
pub mod model;
pub mod platform;
pub mod resources;
#[cfg(windows)]
pub mod security;
#[cfg(unix)]
#[path = "security_unix.rs"]
pub mod security;
pub mod store;
#[cfg(windows)]
pub mod tray;
#[cfg(unix)]
#[path = "tray_unix.rs"]
pub mod tray;
#[cfg(windows)]
pub mod updater;
#[cfg(unix)]
#[path = "updater_unix.rs"]
pub mod updater;
pub mod updates;
pub mod worker;
