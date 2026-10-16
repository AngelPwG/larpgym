//! SQLite persistence, durable outbox, server journal and conflict audit.
use anyhow::{ensure, Context, Result};
use chrono::Utc;
use gym_core::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Client,
    Server,
}
pub struct Store {
    conn: Connection,
    path: PathBuf,
    role: Role,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub entity_id: Uuid,
    pub local: Document,
    pub current: Option<Revision>,
    pub reason: String,
}
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub documents: Vec<Document>,
    pub pending: usize,
    pub conflicts: Vec<Conflict>,
    pub last_sync: Option<String>,
}

pub fn private_file(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            }
        }
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}
pub fn read_secret(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path).context("No se pudo leer el archivo de token")?;
    ensure!(metadata.is_file(), "El token debe ser un archivo regular");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "El token requiere permisos 0600 (chmod 600)"
        );
    }
    ensure!(metadata.len() <= 1024, "Archivo de token demasiado grande");
    let token = fs::read_to_string(path)?.trim().to_owned();
    ensure!(
        (32..=512).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_graphic()),
        "Token inválido: usa al menos 32 caracteres ASCII aleatorios"
    );
    Ok(token)
}
fn decode<T: serde::de::DeserializeOwned>(s: String) -> Result<T> {
    Ok(serde_json::from_str(&s)?)
}
fn get_revision(conn: &Connection, table: &str, id: Uuid) -> Result<Option<Revision>> {
    // Table is always a source-code constant, never input from a request.
    let json: Option<String> = conn
        .query_row(
            &format!("SELECT json FROM {table} WHERE id=?1"),
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    json.map(decode).transpose()
}
fn put_revision(conn: &Connection, table: &str, revision: &Revision) -> Result<()> {
    conn.execute(&format!("INSERT INTO {table}(id,json) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json"),params![revision.document.id.to_string(),serde_json::to_string(revision)?])?;
    Ok(())
}
fn revisions(conn: &Connection, table: &str) -> Result<Vec<Revision>> {
    let mut statement = conn.prepare(&format!("SELECT json FROM {table} ORDER BY id"))?;
    let rows = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(decode).collect()
}
fn meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()?)
}
fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
    Ok(())
}
fn pending_for(conn: &Connection, id: Uuid) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM outbox WHERE entity_id=?1)",
        [id.to_string()],
        |r| r.get(0),
    )?)
}
fn put_shadow(conn: &Connection, revision: &Revision) -> Result<()> {
    if get_revision(conn, "remote", revision.document.id)?
        .is_none_or(|r| r.version <= revision.version)
    {
        put_revision(conn, "remote", revision)?;
    }
    Ok(())
}
fn validate_write(conn: &Connection, doc: &Document) -> Result<()> {
    doc.validate()?;
    if let Some(old) = get_revision(conn, "records", doc.id)? {
        ensure!(
            old.document.kind() == doc.kind(),
            "No se puede cambiar el tipo de un registro"
        );
    }
    let mut docs: Vec<_> = revisions(conn, "records")?
        .into_iter()
        .map(|r| r.document)
        .filter(|d| d.id != doc.id)
        .collect();
    docs.push(doc.clone());
    validate_graph(&docs)
}

impl Store {
    pub fn open(path: impl AsRef<Path>, role: Role) -> Result<Self> {
        let path = path.as_ref().to_owned();
        drop(private_file(&path)?);
        let mut conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_secs(3))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version <= 1, "Base creada por una versión más reciente");
        if version == 0 {
            let tx = conn.transaction()?;
            tx.execute_batch(include_str!("migration-001.sql"))?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()?;
        }
        let expected = if role == Role::Client {
            "client"
        } else {
            "server"
        };
        if let Some(saved) = meta(&conn, "role")? {
            ensure!(
                saved == expected,
                "No uses la misma base para cliente y servidor"
            );
        } else {
            set_meta(&conn, "role", expected)?;
        }
        if role == Role::Client && meta(&conn, "device_id")?.is_none() {
            set_meta(&conn, "device_id", &Uuid::new_v4().to_string())?;
        }
        Ok(Self { conn, path, role })
    }
    pub fn device_id(&self) -> Result<Uuid> {
        Ok(meta(&self.conn, "device_id")?
            .context("Base sin identidad de cliente")?
            .parse()?)
    }
    pub fn documents(&self) -> Result<Vec<Document>> {
        Ok(revisions(&self.conn, "records")?
            .into_iter()
            .map(|r| r.document)
            .collect())
    }
    pub fn document(&self, id: Uuid) -> Result<Option<Document>> {
        Ok(get_revision(&self.conn, "records", id)?.map(|r| r.document))
    }
    pub fn revision(&self, id: Uuid) -> Result<Option<Revision>> {
        get_revision(&self.conn, "records", id)
    }
    pub fn save(&mut self, document: Document) -> Result<()> {
        let expected = self.document(document.id)?;
        self.save_checked(document, expected.as_ref())
    }
    /// Compare the form's original value, so refreshes never silently overwrite a stale form.
    pub fn save_checked(&mut self, document: Document, expected: Option<&Document>) -> Result<()> {
        ensure!(
            self.role == Role::Client,
            "Escritura local en base de servidor"
        );
        let device_id = self.device_id()?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let old = get_revision(&tx, "records", document.id)?;
        ensure!(old.as_ref().map(|r| &r.document)==expected,"El registro cambió mientras editabas. Copia tu propuesta, cierra y vuelve a abrir el formulario");
        if old.as_ref().map(|r| &r.document) == Some(&document) {
            return Ok(());
        }
        validate_write(&tx, &document)?;
        tx.execute(
            "DELETE FROM metadata WHERE key=?1",
            [format!("bootstrap/{}", document.id)],
        )?;
        let base = old.as_ref().map_or(0, |r| r.version);
        let after: Option<String> = tx
            .query_row(
                "SELECT id FROM outbox WHERE entity_id=?1 ORDER BY seq DESC LIMIT 1",
                [document.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let operation = Operation {
            id: Uuid::new_v4(),
            device_id,
            base_version: base,
            strict_version: false,
            document: document.clone(),
        };
        tx.execute(
            "INSERT INTO outbox(id,entity_id,json,after_id,frozen) VALUES(?1,?2,?3,?4,0)",
            params![
                operation.id.to_string(),
                document.id.to_string(),
                serde_json::to_string(&operation)?,
                after
            ],
        )?;
        put_revision(
            &tx,
            "records",
            &Revision {
                document,
                version: base,
                updated_at: Utc::now(),
            },
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn seed_catalogue(&mut self) -> Result<()> {
        if meta(&self.conn, "seeded")?.is_some() {
            return Ok(());
        }
        for d in initial_catalogue() {
            if self.document(d.id)?.is_none() {
                self.save(d.clone())?;
                set_meta(
                    &self.conn,
                    &format!("bootstrap/{}", d.id),
                    &serde_json::to_string(&d)?,
                )?;
            }
        }
        set_meta(&self.conn, "seeded", "1")
    }
    /// One frozen operation per entity; a dependent edit is rebased ONLY onto its own ack.
    pub fn prepare_next(&mut self) -> Result<Option<Operation>> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row: Option<(i64,String)>=tx.query_row("SELECT o.seq,o.json FROM outbox o WHERE NOT EXISTS(SELECT 1 FROM conflicts c WHERE c.entity_id=o.entity_id) AND NOT EXISTS(SELECT 1 FROM outbox p WHERE p.entity_id=o.entity_id AND p.seq<o.seq) ORDER BY o.seq LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let result = if let Some((seq, json)) = row {
            tx.execute("UPDATE outbox SET frozen=1 WHERE seq=?1", [seq])?;
            Some(decode(json)?)
        } else {
            None
        };
        tx.commit()?;
        Ok(result)
    }
    pub fn acknowledge(&mut self, op: &Operation, outcome: &Outcome) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE id=?1)",
            [op.id.to_string()],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(());
        }
        match outcome {
            Outcome::Accepted { revision } => {
                ensure!(
                    revision.document.id == op.document.id
                        && revision.document == op.document
                        && revision.version > 0,
                    "Confirmación inválida del servidor"
                );
                put_shadow(&tx, revision)?;
                tx.execute("DELETE FROM outbox WHERE id=?1", [op.id.to_string()])?;
                let dependents: Vec<(String, String)> = {
                    let mut s =
                        tx.prepare("SELECT id,json FROM outbox WHERE after_id=?1 AND frozen=0")?;
                    let values = s
                        .query_map([op.id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?
                        .collect::<rusqlite::Result<_>>()?;
                    values
                };
                for (id, json) in dependents {
                    let mut next: Operation = decode(json)?;
                    next.base_version = revision.version;
                    tx.execute(
                        "UPDATE outbox SET json=?1,after_id=NULL WHERE id=?2",
                        params![serde_json::to_string(&next)?, id],
                    )?;
                }
                if pending_for(&tx, op.document.id)? {
                    if let Some(mut local) = get_revision(&tx, "records", op.document.id)? {
                        local.version = revision.version;
                        put_revision(&tx, "records", &local)?;
                    }
                } else if let Some(latest) = get_revision(&tx, "remote", op.document.id)? {
                    put_revision(&tx, "records", &latest)?;
                }
            }
            Outcome::Conflict { current, reason } => {
                if let Some(r) = current {
                    ensure!(r.document.id == op.document.id, "Conflicto de ID inválido");
                    put_shadow(&tx, r)?;
                }
                tx.execute("INSERT INTO conflicts(entity_id,reason) VALUES(?1,?2) ON CONFLICT(entity_id) DO UPDATE SET reason=excluded.reason",params![op.document.id.to_string(),reason])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn cursor(&self) -> Result<u64> {
        Ok(meta(&self.conn, "cursor")?
            .unwrap_or_else(|| "0".into())
            .parse()?)
    }
    pub fn apply_page(&mut self, page: &Page) -> Result<()> {
        let previous = self.cursor()?;
        ensure!(
            page.next_cursor >= previous && page.changes.len() <= PAGE_SIZE,
            "Página inválida"
        );
        let mut seen = previous;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for change in &page.changes {
            ensure!(
                change.cursor > seen && change.cursor <= page.next_cursor,
                "Cursor no incremental"
            );
            seen = change.cursor;
            let r = &change.revision;
            r.document.validate()?;
            ensure!(r.version > 0, "Versión inválida");
            put_shadow(&tx, r)?;
            let bootstrap_key = format!("bootstrap/{}", r.document.id);
            if let Some(bootstrap) = meta(&tx, &bootstrap_key)? {
                let unchanged = get_revision(&tx, "records", r.document.id)?.is_some_and(|local| {
                    serde_json::to_string(&local.document).is_ok_and(|j| j == bootstrap)
                });
                if unchanged {
                    tx.execute(
                        "DELETE FROM outbox WHERE entity_id=?1",
                        [r.document.id.to_string()],
                    )?;
                }
                tx.execute("DELETE FROM metadata WHERE key=?1", [bootstrap_key])?;
            }
            if !pending_for(&tx, r.document.id)? {
                let latest = get_revision(&tx, "remote", r.document.id)?.unwrap();
                put_revision(&tx, "records", &latest)?;
            }
        }
        ensure!(
            seen == page.next_cursor && (!page.has_more || !page.changes.is_empty()),
            "Página sin avance"
        );
        set_meta(&tx, "cursor", &page.next_cursor.to_string())?;
        tx.commit()?;
        Ok(())
    }
    pub fn mark_synced(&self) -> Result<()> {
        set_meta(&self.conn, "last_sync", &Utc::now().to_rfc3339())
    }
    pub fn conflicts(&self) -> Result<Vec<Conflict>> {
        let mut s = self
            .conn
            .prepare("SELECT entity_id,reason FROM conflicts ORDER BY entity_id")?;
        let rows = s
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, reason)| {
                let id = id.parse()?;
                Ok(Conflict {
                    entity_id: id,
                    local: self
                        .document(id)?
                        .context("Conflicto sin propuesta local")?,
                    current: get_revision(&self.conn, "remote", id)?,
                    reason,
                })
            })
            .collect()
    }
    /// Save both proposals in the audit, and use the exact revision displayed to the user.
    pub fn resolve(&mut self, shown: &Conflict, keep_local: bool) -> Result<()> {
        let device_id = self.device_id()?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let remote = get_revision(&tx, "remote", shown.entity_id)?;
        ensure!(
            remote == shown.current,
            "El servidor cambió desde que abriste el conflicto; vuelve a revisarlo"
        );
        let local =
            get_revision(&tx, "records", shown.entity_id)?.context("Propuesta inexistente")?;
        ensure!(
            local.document == shown.local,
            "La propuesta local cambió; vuelve a revisar el conflicto"
        );
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM conflicts WHERE entity_id=?1)",
            [shown.entity_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(active, "Conflicto ya resuelto");
        tx.execute(
            "INSERT INTO resolutions(json,resolved_at,choice) VALUES(?1,?2,?3)",
            params![
                serde_json::to_string(shown)?,
                Utc::now().to_rfc3339(),
                if keep_local { "local" } else { "server" }
            ],
        )?;
        tx.execute(
            "DELETE FROM outbox WHERE entity_id=?1",
            [shown.entity_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM conflicts WHERE entity_id=?1",
            [shown.entity_id.to_string()],
        )?;
        let chosen = if keep_local {
            local.document.clone()
        } else if let Some(r) = &remote {
            r.document.clone()
        } else {
            let mut d = local.document.clone();
            d.deleted = true;
            d
        };
        validate_write(&tx, &chosen)?;
        let base = remote.as_ref().map_or(0, |r| r.version);
        let op = Operation {
            id: Uuid::new_v4(),
            device_id,
            base_version: base,
            strict_version: true,
            document: chosen.clone(),
        };
        tx.execute(
            "INSERT INTO outbox(id,entity_id,json,frozen) VALUES(?1,?2,?3,0)",
            params![
                op.id.to_string(),
                chosen.id.to_string(),
                serde_json::to_string(&op)?
            ],
        )?;
        put_revision(
            &tx,
            "records",
            &Revision {
                document: chosen,
                version: base,
                updated_at: Utc::now(),
            },
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot {
            documents: self.documents()?,
            pending: self
                .conn
                .query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get::<_, i64>(0))?
                as usize,
            conflicts: self.conflicts()?,
            last_sync: meta(&self.conn, "last_sync")?,
        })
    }
    pub fn sync_lock(&self) -> Result<File> {
        let lock = private_file(&self.path.with_extension("sync.lockfile"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("Ya hay una sincronización activa para esta base")?;
        Ok(lock)
    }
    pub fn server_apply(&mut self, op: &Operation) -> Result<Outcome> {
        ensure!(self.role == Role::Server, "API en base de cliente");
        ensure!(
            !op.id.is_nil() && !op.device_id.is_nil() && op.base_version < i64::MAX as u64,
            "Identidad o versión inválida"
        );
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let request = serde_json::to_string(op)?;
        let cached: Option<(String, String)> = tx
            .query_row(
                "SELECT request,outcome FROM operations WHERE id=?1",
                [op.id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((original, json)) = cached {
            ensure!(
                original == request,
                "Un ID de operación no puede reutilizarse con otro contenido"
            );
            return decode(json);
        }
        let current = get_revision(&tx, "records", op.document.id)?;
        let outcome =
            if op.strict_version && current.as_ref().map_or(0, |r| r.version) != op.base_version {
                Outcome::Conflict {
                    current,
                    reason: "La versión cambió después de revisar la resolución".into(),
                }
            } else if let Err(e) = validate_write(&tx, &op.document) {
                Outcome::Conflict {
                    current,
                    reason: e.to_string(),
                }
            } else if current.as_ref().map(|r| &r.document) == Some(&op.document) {
                // Identical desired state is safe, including concurrent identical catalogue bootstrap.
                Outcome::Accepted {
                    revision: current.unwrap(),
                }
            } else if current.as_ref().map_or(0, |r| r.version) != op.base_version {
                Outcome::Conflict {
                    current,
                    reason: "Otro dispositivo cambió este registro desde tu versión".into(),
                }
            } else {
                let version = current.as_ref().map_or(0, |r| r.version) + 1;
                let revision = Revision {
                    document: op.document.clone(),
                    version,
                    updated_at: Utc::now(),
                };
                put_revision(&tx, "records", &revision)?;
                tx.execute(
                    "INSERT INTO changes(json) VALUES(?1)",
                    [serde_json::to_string(&revision)?],
                )?;
                Outcome::Accepted { revision }
            };
        tx.execute(
            "INSERT INTO operations(id,request,outcome) VALUES(?1,?2,?3)",
            params![op.id.to_string(), request, serde_json::to_string(&outcome)?],
        )?;
        tx.commit()?;
        Ok(outcome)
    }
    pub fn server_page(&self, cursor: u64, limit: usize) -> Result<Page> {
        ensure!(
            self.role == Role::Server
                && cursor <= i64::MAX as u64
                && (1..=PAGE_SIZE).contains(&limit),
            "Cursor o límite inválido"
        );
        let max: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(seq),0) FROM changes", [], |r| r.get(0))?;
        ensure!(
            cursor <= max as u64,
            "Cursor posterior al servidor; comprueba si restauraste una copia antigua"
        );
        let mut s = self
            .conn
            .prepare("SELECT seq,json FROM changes WHERE seq>?1 ORDER BY seq LIMIT ?2")?;
        let rows = s
            .query_map(params![cursor as i64, limit as i64], |r| {
                Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let changes = rows
            .into_iter()
            .map(|(cursor, json)| {
                Ok(Change {
                    cursor,
                    revision: decode(json)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = changes.last().map_or(cursor, |c| c.cursor);
        Ok(Page {
            changes,
            next_cursor,
            has_more: next_cursor < max as u64,
        })
    }
    pub fn backup(&self, destination: &Path) -> Result<()> {
        ensure!(
            !destination.exists(),
            "El destino ya existe; no se sobrescribirá"
        );
        if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(options.open(destination)?);
        if let Err(error) = self.conn.backup("main", destination, None) {
            let _ = fs::remove_file(destination);
            return Err(error.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    #[test]
    fn stale_forms_backup_and_archived_history() {
        let tmp = TempDir::new().unwrap();
        let mut s = Store::open(tmp.path().join("client"), Role::Client).unwrap();
        s.seed_catalogue().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let original = Document::day(date);
        s.save(original.clone()).unwrap();
        let mut changed = original.clone();
        if let Entity::Day(d) = &mut changed.entity {
            d.weight = Some(70000);
        }
        s.save(changed.clone()).unwrap();
        assert!(s
            .save_checked(original, Some(&Document::day(date)))
            .is_err());
        let dest = tmp.path().join("backup");
        s.backup(&dest).unwrap();
        let restored = Store::open(&dest, Role::Client).unwrap();
        assert_eq!(s.device_id().unwrap(), restored.device_id().unwrap());
        assert_eq!(
            s.snapshot().unwrap().pending,
            restored.snapshot().unwrap().pending
        );
        assert_eq!(restored.document(changed.id).unwrap(), Some(changed));
        assert!(s.backup(&dest).is_err());
        assert!(Store::open(dest, Role::Server).is_err());
    }
}
