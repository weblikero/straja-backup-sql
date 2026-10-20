//! Native Keychain / Secret Service storage. SQLite contains opaque references only.
use anyhow::{Context, Result, ensure};
const SERVICE: &str = "com.strajabackup.sql";
const PREFIX: &str = "straja-native-secret-v1:";
pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
    let id = uuid::Uuid::new_v4().to_string();
    keyring::Entry::new(SERVICE, &id)?
        .set_secret(data)
        .context("Unlock the system credential store before saving credentials")?;
    Ok(format!("{PREFIX}{id}").into_bytes())
}
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    let reference = std::str::from_utf8(data)?
        .strip_prefix(PREFIX)
        .context("Credentials from another OS cannot be imported; enter them again")?;
    uuid::Uuid::parse_str(reference)?;
    ensure!(reference.len() == 36, "Invalid credential reference");
    Ok(keyring::Entry::new(SERVICE, reference)?
        .get_secret()
        .context("Unlock the system credential store to access backup credentials")?)
}
// Exclusive lifetime file locks in main/worker provide single-instance protection.
pub struct AppMutex;
impl AppMutex {
    pub fn new(_: &str) -> Result<Self> {
        Ok(Self)
    }
}
