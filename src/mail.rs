use crate::i18n::tr;
use crate::model::MailSettings;
use anyhow::{Result, bail};
use lettre::{
    Message, SmtpTransport, Transport, message::Mailbox,
    transport::smtp::authentication::Credentials,
};
pub fn validate(m: &MailSettings) -> Result<()> {
    if m.host.trim().is_empty() || m.port == 0 {
        bail!("{}", tr("Enter the SMTP server and port."));
    }
    let _: Mailbox = m.from.parse()?;
    let _: Mailbox = m.to.parse()?;
    Ok(())
}
pub fn send(m: &MailSettings, password: &str, subject: &str, body: &str) -> Result<()> {
    validate(m)?;
    let email = Message::builder()
        .from(m.from.parse()?)
        .to(m.to.parse()?)
        .subject(subject)
        .body(body.to_owned())?;
    let mut transport = if m.implicit_tls {
        SmtpTransport::relay(&m.host)?
    } else {
        SmtpTransport::starttls_relay(&m.host)?
    }
    .port(m.port)
    .timeout(Some(std::time::Duration::from_secs(30)));
    if !m.username.is_empty() {
        transport =
            transport.credentials(Credentials::new(m.username.clone(), password.to_string()));
    }
    transport.build().send(&email)?;
    Ok(())
}
