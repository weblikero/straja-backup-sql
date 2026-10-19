use crate::i18n::{self, tr};
use crate::{engine, mail, model::*, security, store::Store};
use anyhow::Result;
use chrono::Utc;
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn execute(root: &Path, id: &str, plan_id: &str) -> Result<()> {
    let _background = crate::resources::BackgroundWork::enter();
    let store = Store::open(root)?;
    let plan: Plan = store.get("plan", plan_id)?;
    let result = (|| {
        let connection: Connection = store.get("connection", &plan.connection)?;
        let password = store.secret(&connection.id)?;
        engine::backup_with_progress(&plan, &connection, &password, id, &mut |progress| {
            let _ = store.put("progress", id, &progress);
        })
    })();
    match result {
        Ok(folder) => {
            let _ = store.put(
                "progress",
                id,
                &BackupProgress {
                    phase: "Retenție".into(),
                    database: String::new(),
                    database_index: plan.databases.len(),
                    database_count: plan.databases.len(),
                    bytes: 0,
                    total_bytes: None,
                    updated: Utc::now(),
                },
            );
            let detail = match engine::retention(&plan, &folder, Utc::now()) {
                Ok(n) => i18n::format(
                    "Databases exported: {0}. SHA-256 and reread verified. Expired copies removed: {1}. Verification does not include a restore on a test server.",
                    &[(plan.databases.len()).to_string(), (n).to_string()],
                ),
                Err(e) => i18n::format(
                    "Backup verified; retention needs attention: {0}",
                    &[format!("{:#}", e)],
                ),
            };
            store.finish(id, "Reușit", &detail, Some(&folder))?;
        }
        Err(e) => {
            let detail = format!("{e:#}");
            // Persist failure before trying SMTP, so a mail outage cannot hide a backup failure.
            store.finish(id, "Eroare", &detail, None)?;
            if plan.notify {
                let sent = (|| -> Result<()> {
                    let settings = store.get("settings", "mail")?;
                    let password = store.secret("smtp")?;
                    mail::send(
                        &settings,
                        &password,
                        &i18n::format(
                            "StrajaBackupSQL · backup failed: {0}",
                            std::slice::from_ref(&plan.name),
                        ),
                        &i18n::format(
                            "Plan: {0}\nTime (UTC): {1}\nError: {2}\n\nThe app has not modified the source databases.",
                            &[
                                (plan.name).to_string(),
                                (Utc::now()).to_string(),
                                (detail).to_string(),
                            ],
                        ),
                    )
                })();
                let notification = match sent {
                    Ok(()) => tr("Alert email sent.").to_string(),
                    Err(e) => i18n::format("Alert email was NOT sent: {0}", &[format!("{:#}", e)]),
                };
                store.finish(id, "Eroare", &format!("{detail}\n{notification}"), None)?;
            }
        }
    }
    Ok(())
}
pub fn run(root: PathBuf) -> Result<()> {
    i18n::reload(&root);
    std::fs::create_dir_all(&root)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("agent.lock"))?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    let _mutex = security::AppMutex::new("Local\\StrajaBackupSQL.Agent")?;
    let store = Store::open(&root)?;
    store.recover()?;
    store.put("agent", "stop", &false)?;
    // The icon belongs to the background agent, not the Iced window.
    // A temporarily unavailable Explorer must not disable scheduled backups.
    let _tray = match crate::tray::Tray::start(root.clone()) {
        Ok(tray) => {
            store.put("agent", "tray_error", &String::new())?;
            Some(tray)
        }
        Err(error) => {
            store.put("agent", "tray_error", &format!("{error:#}"))?;
            None
        }
    };
    let mut active: Option<std::thread::JoinHandle<()>> = None;
    let mut last_heartbeat = None::<Instant>;
    loop {
        i18n::reload_from(&store);
        if last_heartbeat.is_none_or(|last| last.elapsed() >= Duration::from_secs(10)) {
            store.put("agent", "heartbeat", &Utc::now())?;
            last_heartbeat = Some(Instant::now());
        }
        if active.as_ref().is_some_and(|h| h.is_finished())
            && let Some(h) = active.take()
        {
            let _ = h.join();
        }
        if active.is_none() {
            if store.get::<bool>("agent", "stop").unwrap_or(false) {
                break;
            }
            store.tick_schedule(Utc::now())?;
            if let Some((id, plan)) = store.claim()? {
                let root = root.clone();
                active = Some(std::thread::spawn(move || {
                    let outcome = std::panic::catch_unwind(|| execute(&root, &id, &plan));
                    let error = match outcome {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(format!("{e:#}")),
                        Err(_) => Some(tr("Internal agent error").into()),
                    };
                    if let Some(error) = error
                        && let Ok(s) = Store::open(&root)
                    {
                        let _ = s.finish(&id, "Eroare", &error, None);
                    }
                }));
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    store.db.execute(
        "DELETE FROM objects WHERE kind='agent' AND id='heartbeat'",
        [],
    )?;
    Ok(())
}
pub fn launch() -> Result<()> {
    let agent = std::env::current_exe()?.with_file_name("StrajaBackupSQL-Agent.exe");
    engine::command(agent).spawn()?;
    Ok(())
}
