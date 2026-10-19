use crate::i18n::tr;
use crate::{model::*, security};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection as Db, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub fn data_dir() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context(tr("LOCALAPPDATA is missing"))?)
            .join("StrajaSQL"),
    )
}
pub struct Store {
    pub db: Db,
}
impl Store {
    pub fn request_window(&self, activity: bool) -> Result<()> {
        self.put("ui", "open", &activity)
    }
    pub fn take_window_request(&self) -> Result<Option<bool>> {
        let value: Option<String> = self
            .db
            .query_row(
                "DELETE FROM objects WHERE kind='ui' AND id='open' RETURNING json",
                [],
                |r| r.get(0),
            )
            .optional()?;
        value.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let db = Db::open(root.join("straja-sql.sqlite"))?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS objects(kind TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,PRIMARY KEY(kind,id));
            CREATE TABLE IF NOT EXISTS secrets(id TEXT PRIMARY KEY,value BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY,plan_id TEXT NOT NULL,plan_name TEXT NOT NULL,status TEXT NOT NULL,started TEXT NOT NULL,finished TEXT,detail TEXT NOT NULL,folder TEXT);
            CREATE UNIQUE INDEX IF NOT EXISTS active_plan ON runs(plan_id) WHERE status IN ('În așteptare','În lucru');")?;
        Ok(Self { db })
    }
    pub fn put<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<()> {
        self.db.execute("INSERT INTO objects VALUES (?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET json=excluded.json", params![kind,id,serde_json::to_string(value)?])?;
        Ok(())
    }
    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<T> {
        let s: String = self.db.query_row(
            "SELECT json FROM objects WHERE kind=?1 AND id=?2",
            params![kind, id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&s)?)
    }
    pub fn list<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        let mut s = self
            .db
            .prepare("SELECT json FROM objects WHERE kind=?1 ORDER BY rowid")?;
        let rows = s.query_map([kind], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn secret(&self, id: &str) -> Result<Zeroizing<String>> {
        let bytes: Vec<u8> =
            self.db
                .query_row("SELECT value FROM secrets WHERE id=?1", [id], |r| r.get(0))?;
        let plain = Zeroizing::new(security::unprotect(&bytes)?);
        Ok(Zeroizing::new(String::from_utf8(plain.to_vec())?))
    }
    fn put_secret(&self, id: &str, value: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO secrets VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET value=excluded.value",
            params![id, security::protect(value.as_bytes())?],
        )?;
        Ok(())
    }
    pub fn save_connection(&mut self, c: &Connection, password: &str) -> Result<()> {
        c.validate()?;
        let tx = self.db.unchecked_transaction()?;
        self.put("connection", &c.id, c)?;
        if !password.is_empty() || self.secret(&c.id).is_err() {
            self.put_secret(&c.id, password)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn save_mail(&self, m: &MailSettings, password: &str) -> Result<()> {
        crate::mail::validate(m)?;
        let tx = self.db.unchecked_transaction()?;
        self.put("settings", "mail", m)?;
        if !password.is_empty() || self.secret("smtp").is_err() {
            self.put_secret("smtp", password)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn save_plan(&self, p: &Plan) -> Result<()> {
        p.validate()?;
        let _: Connection = self.get("connection", &p.connection)?;
        if self.busy(&p.id)? {
            bail!("{}", tr("This plan has a running or queued backup."));
        }
        self.put("plan", &p.id, p)
    }
    pub fn busy(&self, id: &str) -> Result<bool> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE plan_id=?1 AND status IN ('În așteptare','În lucru'))", [id], |r| r.get(0))?)
    }
    pub fn delete(&self, kind: &str, id: &str) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        if kind == "connection"
            && self
                .list::<Plan>("plan")?
                .iter()
                .any(|p| p.connection == id)
        {
            bail!(
                "{}",
                tr("A plan uses this connection. Remove the plan first.")
            );
        }
        if kind == "plan" && self.busy(id)? {
            bail!("{}", tr("Wait for the current run to finish."));
        }
        self.db.execute(
            "DELETE FROM objects WHERE kind=?1 AND id=?2",
            params![kind, id],
        )?;
        if kind == "connection" {
            self.db.execute("DELETE FROM secrets WHERE id=?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn enqueue(&self, plan: &Plan) -> Result<()> {
        if self.busy(&plan.id)? {
            bail!("{}", tr("This plan is already queued or running."));
        }
        self.db.execute("INSERT INTO runs(id,plan_id,plan_name,status,started,detail) VALUES (?1,?2,?3,'În așteptare',?4,?5)", params![uuid::Uuid::new_v4().to_string(),plan.id,plan.name,Utc::now().to_rfc3339(),tr("Waiting for agent")])?;
        Ok(())
    }
    pub fn claim(&self) -> Result<Option<(String, String)>> {
        let result = self.db.query_row("UPDATE runs SET status='În lucru',detail=?2,started=?1 WHERE id=(SELECT id FROM runs WHERE status='În așteptare' ORDER BY started LIMIT 1) RETURNING id,plan_id", params![Utc::now().to_rfc3339(),tr("Export and verification")], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        Ok(result)
    }
    pub fn finish(
        &self,
        id: &str,
        status: &str,
        detail: &str,
        folder: Option<&Path>,
    ) -> Result<()> {
        self.db.execute(
            "UPDATE runs SET status=?2,detail=?3,finished=?4,folder=?5 WHERE id=?1",
            params![
                id,
                status,
                detail,
                Utc::now().to_rfc3339(),
                folder.map(|p| p.to_string_lossy().to_string())
            ],
        )?;
        Ok(())
    }
    pub fn recover(&self) -> Result<()> {
        self.db.execute(
            "UPDATE runs SET status='Întrerupt',detail=?2,finished=?1 WHERE status='În lucru'",
            params![
                Utc::now().to_rfc3339(),
                tr("Agent stopped before completion. The partial backup is not validated.")
            ],
        )?;
        Ok(())
    }
    pub fn runs(&self) -> Result<Vec<Run>> {
        let mut s = self.db.prepare("SELECT id,plan_id,plan_name,status,started,finished,detail,folder FROM runs ORDER BY started DESC LIMIT 200")?;
        Ok(s.query_map([], |r| {
            Ok(Run {
                id: r.get(0)?,
                plan_id: r.get(1)?,
                plan_name: r.get(2)?,
                status: r.get(3)?,
                started: r.get(4)?,
                finished: r.get(5)?,
                detail: r.get(6)?,
                folder: r.get::<_, Option<String>>(7)?.map(PathBuf::from),
                progress: self.get("progress", &r.get::<_, String>(0)?).ok(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot {
            connections: self.list("connection")?,
            plans: self.list("plan")?,
            runs: self.runs()?,
            heartbeat: self.get::<DateTime<Utc>>("agent", "heartbeat").ok(),
            mail: self.get("settings", "mail").unwrap_or_default(),
        })
    }
    pub fn tick_schedule(&self, now: DateTime<Utc>) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        for mut p in self.list::<Plan>("plan")? {
            if p.enabled && p.next_due.is_some_and(|d| d <= now) && !self.busy(&p.id)? {
                self.enqueue(&p)?;
                p.next_due = p.schedule.next_after(now);
                self.put("plan", &p.id, &p)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
