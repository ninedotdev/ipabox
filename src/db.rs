use std::sync::Mutex;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::models::{InstalledApp, PairedDevice};
use crate::paths;

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open() -> Result<Self> {
        let conn = Connection::open(paths::db_path())?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS installed_apps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ipa_name TEXT, ipa_path TEXT, device TEXT, device_class TEXT,
                udid TEXT, account TEXT, password TEXT,
                installed_date TEXT, refreshed_date TEXT, expiration_date TEXT,
                refreshed_result INTEGER DEFAULT 0, refreshed_error INTEGER DEFAULT 0,
                icon TEXT, bundle_identifier TEXT, version TEXT,
                remove_extensions INTEGER DEFAULT 0, enabled INTEGER DEFAULT 1,
                created_at TEXT
            );
            CREATE TABLE IF NOT EXISTS paired_devices (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT, device_class TEXT, udid TEXT, ip TEXT,
                pair_type TEXT, last_paired_at TEXT,
                UNIQUE(name, device_class)
            );
            "#,
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // ---- installed_apps -------------------------------------------------

    pub fn list_apps(&self) -> Result<Vec<InstalledApp>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, ipa_name, ipa_path, device, device_class, udid, account, password, \
             installed_date, refreshed_date, expiration_date, refreshed_result, refreshed_error, \
             icon, bundle_identifier, version, remove_extensions, enabled, created_at \
             FROM installed_apps ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], row_to_app)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn enabled_apps(&self) -> Result<Vec<InstalledApp>> {
        Ok(self
            .list_apps()?
            .into_iter()
            .filter(|a| a.enabled)
            .collect())
    }

    pub fn get_app(&self, id: i64) -> Result<Option<InstalledApp>> {
        let conn = self.conn.lock().unwrap();
        let app = conn
            .query_row(
                "SELECT id, ipa_name, ipa_path, device, device_class, udid, account, password, \
                 installed_date, refreshed_date, expiration_date, refreshed_result, refreshed_error, \
                 icon, bundle_identifier, version, remove_extensions, enabled, created_at \
                 FROM installed_apps WHERE id = ?1",
                params![id],
                row_to_app,
            )
            .optional()?;
        Ok(app)
    }

    /// Insert or update by the (device, class, bundle, account) dedupe key —
    /// mirrors `service.SaveApp`. Returns the row id.
    pub fn save_app(&self, app: &InstalledApp) -> Result<i64> {
        let existing = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT id FROM installed_apps WHERE device=?1 AND device_class=?2 AND bundle_identifier=?3 AND account=?4",
                params![app.device, app.device_class, app.bundle_identifier, app.account],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        };

        let now = Utc::now();
        if let Some(id) = existing {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE installed_apps SET ipa_path=?1, icon=?2, version=?3, refreshed_date=?4, \
                 expiration_date=?5, refreshed_result=?6, refreshed_error=?7, password=?8 WHERE id=?9",
                params![
                    app.ipa_path,
                    app.icon,
                    app.version,
                    dt(Some(now)),
                    dt(app.expiration_date),
                    app.refreshed_result as i64,
                    app.refreshed_error,
                    app.password,
                    id
                ],
            )?;
            Ok(id)
        } else {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO installed_apps (ipa_name, ipa_path, device, device_class, udid, account, \
                 password, installed_date, refreshed_date, expiration_date, refreshed_result, \
                 refreshed_error, icon, bundle_identifier, version, remove_extensions, enabled, created_at) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,1,?17)",
                params![
                    app.ipa_name, app.ipa_path, app.device, app.device_class, app.udid, app.account,
                    app.password, dt(Some(now)), dt(app.refreshed_date), dt(app.expiration_date),
                    app.refreshed_result as i64, app.refreshed_error, app.icon, app.bundle_identifier,
                    app.version, app.remove_extensions as i64, dt(Some(now)),
                ],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }

    pub fn update_paths(&self, id: i64, ipa_path: &str, icon: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE installed_apps SET ipa_path=?1, icon=?2 WHERE id=?3",
            params![ipa_path, icon, id],
        )?;
        Ok(())
    }

    pub fn update_refresh_result(&self, app: &InstalledApp) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE installed_apps SET refreshed_date=?1, expiration_date=?2, refreshed_result=?3, \
             refreshed_error=?4 WHERE id=?5",
            params![
                dt(app.refreshed_date),
                dt(app.expiration_date),
                app.refreshed_result as i64,
                app.refreshed_error,
                app.id
            ],
        )?;
        Ok(())
    }

    pub fn toggle_enabled(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let cur: bool = conn.query_row(
            "SELECT enabled FROM installed_apps WHERE id=?1",
            params![id],
            |r| r.get::<_, i64>(0).map(|v| v != 0),
        )?;
        let next = !cur;
        conn.execute(
            "UPDATE installed_apps SET enabled=?1 WHERE id=?2",
            params![next as i64, id],
        )?;
        Ok(next)
    }

    pub fn delete_app(&self, id: i64) -> Result<Option<String>> {
        let ipa_path = self.get_app(id)?.map(|a| a.ipa_path);
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM installed_apps WHERE id=?1", params![id])?;
        Ok(ipa_path)
    }

    // ---- paired_devices -------------------------------------------------

    pub fn list_paired(&self) -> Result<Vec<PairedDevice>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, device_class, udid, ip, pair_type, last_paired_at \
             FROM paired_devices ORDER BY last_paired_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PairedDevice {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    device_class: r.get(2)?,
                    udid: r.get(3)?,
                    ip: r.get(4)?,
                    pair_type: r.get(5)?,
                    last_paired_at: parse_dt(r.get::<_, Option<String>>(6)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn save_paired(&self, name: &str, device_class: &str, ip: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO paired_devices (name, device_class, udid, ip, pair_type, last_paired_at) \
             VALUES (?1,?2,'',?3,'remote',?4) \
             ON CONFLICT(name, device_class) DO UPDATE SET ip=?3, pair_type='remote', last_paired_at=?4",
            params![name, device_class, ip, dt(Some(Utc::now()))],
        )?;
        Ok(())
    }

    pub fn delete_paired(&self, name: &str, device_class: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM paired_devices WHERE name=?1 AND device_class=?2",
            params![name, device_class],
        )?;
        Ok(())
    }
}

fn dt(value: Option<DateTime<Utc>>) -> Option<String> {
    value.map(|d| d.to_rfc3339())
}

fn parse_dt(value: Option<String>) -> Option<DateTime<Utc>> {
    value
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn row_to_app(r: &rusqlite::Row) -> rusqlite::Result<InstalledApp> {
    Ok(InstalledApp {
        id: r.get(0)?,
        ipa_name: r.get(1)?,
        ipa_path: r.get(2)?,
        device: r.get(3)?,
        device_class: r.get(4)?,
        udid: r.get(5)?,
        account: r.get(6)?,
        password: r.get(7)?,
        installed_date: parse_dt(r.get(8)?),
        refreshed_date: parse_dt(r.get(9)?),
        expiration_date: parse_dt(r.get(10)?),
        refreshed_result: r.get::<_, i64>(11)? != 0,
        refreshed_error: r.get(12)?,
        icon: r.get(13)?,
        bundle_identifier: r.get(14)?,
        version: r.get(15)?,
        remove_extensions: r.get::<_, i64>(16)? != 0,
        enabled: r.get::<_, i64>(17)? != 0,
        created_at: parse_dt(r.get(18)?),
        team_id: String::new(),
    })
}

/// After a successful RemoteXPC install, persist the record with a fresh 7-day
/// window and move the staged IPA/icon into `ipa/<id>/` — mirrors
/// `service.SaveRemoteInstalledApp` + the file-move half of `SaveApp`.
pub fn save_remote_installed(db: &Db, app: &mut InstalledApp) -> Result<i64> {
    let now = Utc::now();
    app.installed_date = Some(now);
    app.refreshed_date = Some(now);
    app.expiration_date = Some(now + Duration::days(7));
    app.refreshed_result = true;
    app.refreshed_error = crate::models::REFRESH_ERROR_NONE;
    app.enabled = true;
    if app.device_class.is_empty() {
        app.device_class = crate::models::CLASS_APPLE_TV.into();
    }
    if app.ipa_name.is_empty() && !app.ipa_path.is_empty() {
        app.ipa_name = std::path::Path::new(&app.ipa_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
    }

    let id = db.save_app(app)?;
    app.id = id;

    // Move staged files into the per-app directory.
    let save_dir = paths::ipa_dir(id);
    let mut new_ipa = app.ipa_path.clone();
    let mut new_icon = app.icon.clone();
    if !app.ipa_path.is_empty() {
        let dst = save_dir.join("app.ipa");
        if std::fs::rename(&app.ipa_path, &dst).is_ok() {
            new_ipa = dst.display().to_string();
        }
    }
    if !app.icon.is_empty() {
        let dst = save_dir.join("app.png");
        if std::fs::rename(&app.icon, &dst).is_ok() {
            new_icon = dst.display().to_string();
        }
    }
    db.update_paths(id, &new_ipa, &new_icon)?;
    app.ipa_path = new_ipa;
    app.icon = new_icon;
    Ok(id)
}

/// Keep only the newest row per dedupe key (mirrors `CleanupDuplicateApps`).
pub fn cleanup_duplicates(db: &Db) -> Result<usize> {
    let apps = db.list_apps()?;
    let mut seen = std::collections::HashSet::new();
    let mut deleted = 0;
    for a in apps {
        if !seen.insert(a.dedupe_key()) {
            if let Ok(Some(path)) = db.delete_app(a.id) {
                if !path.is_empty() {
                    if let Some(dir) = std::path::Path::new(&path).parent() {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                }
                deleted += 1;
            }
        }
    }
    Ok(deleted)
}
