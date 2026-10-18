//! Historial en SQLite (5.6): modo WAL, cuerpos comprimidos con zstd y limitados a 5 MB.

use std::{path::Path, sync::Mutex, time::Duration};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

/// Límite de cada cuerpo antes de comprimir.
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const ZSTD_LEVEL: i32 = 3;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS history (
  id TEXT PRIMARY KEY, ts INTEGER NOT NULL, request_id TEXT, source TEXT CHECK(source IN ('ui','mcp','cli')),
  method TEXT, url TEXT,
  status INTEGER, duration_ms INTEGER, size_bytes INTEGER,
  req_headers TEXT, req_body BLOB, res_headers TEXT, res_body BLOB,
  tests_json TEXT, error TEXT
);
CREATE INDEX IF NOT EXISTS history_ts ON history(ts DESC);
CREATE TABLE IF NOT EXISTS snapshots (
  request_id TEXT PRIMARY KEY, history_id TEXT, ts INTEGER NOT NULL,
  status INTEGER, content_type TEXT, body BLOB
);
";

const SUMMARY_COLUMNS: &str =
    "id, ts, request_id, source, method, url, status, duration_ms, size_bytes, error";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Ui,
    Mcp,
    Cli,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Ui => "ui",
            Source::Mcp => "mcp",
            Source::Cli => "cli",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "mcp" => Source::Mcp,
            "cli" => Source::Cli,
            _ => Source::Ui,
        }
    }
}

pub type Headers = Vec<(String, String)>;

/// Fila completa. Los cuerpos van sin comprimir aquí; `HistoryDb` comprime al guardar.
/// Quien inserta es responsable de pasar URL, cabeceras y cuerpos ya enmascarados.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    /// Milisegundos desde epoch (UTC).
    pub ts: i64,
    pub request_id: Option<String>,
    pub source: Source,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub duration_ms: u64,
    pub size_bytes: u64,
    pub req_headers: Headers,
    pub req_body: Option<Vec<u8>>,
    pub res_headers: Headers,
    pub res_body: Option<Vec<u8>>,
    pub tests_json: Option<String>,
    pub error: Option<String>,
}

/// Respuesta de referencia de una petición guardada (sección 19.7), ya enmascarada.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub request_id: String,
    /// Ejecución de la que salió (puede haberse borrado del historial después).
    pub history_id: String,
    pub ts: i64,
    pub status: Option<u16>,
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySummary {
    pub id: String,
    pub ts: i64,
    pub request_id: Option<String>,
    pub source: Source,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub duration_ms: u64,
    pub size_bytes: u64,
    pub error: Option<String>,
}

pub struct HistoryDb {
    conn: Mutex<Connection>,
}

impl std::fmt::Debug for HistoryDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HistoryDb")
    }
}

fn compress(body: &Option<Vec<u8>>) -> Result<Option<Vec<u8>>> {
    body.as_deref()
        .map(|bytes| {
            let limited = &bytes[..bytes.len().min(MAX_BODY_BYTES)];
            zstd::bulk::compress(limited, ZSTD_LEVEL).context("zstd compress")
        })
        .transpose()
}

fn decompress(blob: Option<Vec<u8>>) -> rusqlite::Result<Option<Vec<u8>>> {
    blob.map(|bytes| {
        zstd::decode_all(bytes.as_slice()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, error.into())
        })
    })
    .transpose()
}

fn headers_from(text: Option<String>) -> Headers {
    text.and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn summary(row: &Row<'_>) -> rusqlite::Result<HistorySummary> {
    Ok(HistorySummary {
        id: row.get(0)?,
        ts: row.get(1)?,
        request_id: row.get(2)?,
        source: Source::parse(&row.get::<_, Option<String>>(3)?.unwrap_or_default()),
        method: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        url: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        status: row.get::<_, Option<i64>>(6)?.map(|s| s as u16),
        duration_ms: row.get::<_, Option<i64>>(7)?.unwrap_or_default() as u64,
        size_bytes: row.get::<_, Option<i64>>(8)?.unwrap_or_default() as u64,
        error: row.get(9)?,
    })
}

impl HistoryDb {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("cannot open history {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // La app y `runpost mcp` pueden escribir a la vez: WAL + espera ante bloqueo.
        conn.busy_timeout(Duration::from_secs(5))?;
        let _mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "wal", |row| row.get(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn journal_mode(&self) -> Result<String> {
        Ok(self
            .conn()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?)
    }

    pub fn insert(&self, entry: &HistoryEntry) -> Result<()> {
        let req_body = compress(&entry.req_body)?;
        let res_body = compress(&entry.res_body)?;
        self.conn().execute(
            "INSERT INTO history (id, ts, request_id, source, method, url, status, duration_ms, size_bytes,
               req_headers, req_body, res_headers, res_body, tests_json, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                entry.id,
                entry.ts,
                entry.request_id,
                entry.source.as_str(),
                entry.method,
                entry.url,
                entry.status.map(i64::from),
                entry.duration_ms as i64,
                entry.size_bytes as i64,
                serde_json::to_string(&entry.req_headers)?,
                req_body,
                serde_json::to_string(&entry.res_headers)?,
                res_body,
                entry.tests_json,
                entry.error,
            ],
        )?;
        Ok(())
    }

    /// Guarda (o sustituye) la referencia de una petición.
    pub fn save_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        let body = compress(&Some(snapshot.body.clone()))?;
        self.conn().execute(
            "INSERT OR REPLACE INTO snapshots (request_id, history_id, ts, status, content_type, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot.request_id,
                snapshot.history_id,
                snapshot.ts,
                snapshot.status.map(i64::from),
                snapshot.content_type,
                body,
            ],
        )?;
        Ok(())
    }

    pub fn snapshot(&self, request_id: &str) -> Result<Option<Snapshot>> {
        let conn = self.conn();
        let snapshot = conn
            .query_row(
                "SELECT request_id, history_id, ts, status, content_type, body
                 FROM snapshots WHERE request_id = ?1",
                [request_id],
                |row| {
                    Ok(Snapshot {
                        request_id: row.get(0)?,
                        history_id: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        ts: row.get(2)?,
                        status: row.get::<_, Option<i64>>(3)?.map(|s| s as u16),
                        content_type: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        body: decompress(row.get(5)?)?.unwrap_or_default(),
                    })
                },
            )
            .optional()?;
        Ok(snapshot)
    }

    pub fn delete_snapshot(&self, request_id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM snapshots WHERE request_id = ?1", [request_id])?
            > 0)
    }

    /// Más recientes primero. `q` filtra por subcadena de URL o método.
    pub fn list(
        &self,
        limit: usize,
        offset: usize,
        q: Option<&str>,
    ) -> Result<Vec<HistorySummary>> {
        let pattern = q.filter(|q| !q.is_empty()).map(|q| {
            let escaped = q
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("%{escaped}%")
        });
        let conn = self.conn();
        let mut statement = conn.prepare(&format!(
            "SELECT {SUMMARY_COLUMNS} FROM history
             WHERE ?1 IS NULL OR url LIKE ?1 ESCAPE '\\' OR method LIKE ?1 ESCAPE '\\'
             ORDER BY ts DESC, id DESC LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = statement.query_map(params![pattern, limit as i64, offset as i64], summary)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get(&self, id: &str) -> Result<Option<HistoryEntry>> {
        let conn = self.conn();
        let entry = conn
            .query_row(
                &format!(
                    "SELECT {SUMMARY_COLUMNS}, req_headers, req_body, res_headers, res_body, tests_json
                     FROM history WHERE id = ?1"
                ),
                [id],
                |row| {
                    let head = summary(row)?;
                    Ok(HistoryEntry {
                        id: head.id,
                        ts: head.ts,
                        request_id: head.request_id,
                        source: head.source,
                        method: head.method,
                        url: head.url,
                        status: head.status,
                        duration_ms: head.duration_ms,
                        size_bytes: head.size_bytes,
                        error: head.error,
                        req_headers: headers_from(row.get(10)?),
                        req_body: decompress(row.get(11)?)?,
                        res_headers: headers_from(row.get(12)?),
                        res_body: decompress(row.get(13)?)?,
                        tests_json: row.get(14)?,
                    })
                },
            )
            .optional()?;
        Ok(entry)
    }

    /// Borra todo y devuelve cuántas filas había.
    pub fn clear(&self) -> Result<usize> {
        Ok(self.conn().execute("DELETE FROM history", [])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, ts: i64, url: &str) -> HistoryEntry {
        HistoryEntry {
            id: id.into(),
            ts,
            request_id: Some("req_1".into()),
            source: Source::Mcp,
            method: "GET".into(),
            url: url.into(),
            status: Some(200),
            duration_ms: 12,
            size_bytes: 3,
            req_headers: vec![("Authorization".into(), "Bearer ***".into())],
            req_body: Some(b"{\"a\":1}".to_vec()),
            res_headers: vec![
                ("set-cookie".into(), "a=1".into()),
                ("set-cookie".into(), "b=2".into()),
            ],
            res_body: Some(b"pong".to_vec()),
            tests_json: None,
            error: None,
        }
    }

    #[test]
    fn insert_get_list_clear() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db = HistoryDb::open(&dir.path().join("history.db"))?;
        assert_eq!(db.journal_mode()?, "wal");
        db.insert(&entry("h_1", 1, "http://a/ping"))?;
        db.insert(&entry("h_2", 2, "http://b/100%_done"))?;
        let mut failed = entry("h_3", 3, "http://c/x");
        failed.status = None;
        failed.res_body = None;
        failed.error = Some("connect: refused".into());
        failed.source = Source::Cli;
        db.insert(&failed)?;

        let all = db.list(10, 0, None)?;
        assert_eq!(
            all.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
            ["h_3", "h_2", "h_1"]
        );
        assert_eq!(db.list(1, 1, None)?[0].id, "h_2");
        assert_eq!(db.list(10, 0, Some("ping"))?.len(), 1);
        assert_eq!(db.list(10, 0, Some("%_"))?[0].id, "h_2");
        assert_eq!(
            db.list(10, 0, Some("get"))?.len(),
            3,
            "LIKE is case-insensitive on ASCII"
        );
        assert_eq!(db.list(10, 0, Some(""))?.len(), 3);

        let got = db.get("h_1")?.unwrap();
        assert_eq!(got, entry("h_1", 1, "http://a/ping"));
        let got = db.get("h_3")?.unwrap();
        assert_eq!(
            (got.status, got.res_body, got.source),
            (None, None, Source::Cli)
        );
        assert_eq!(db.get("missing")?, None);
        assert_eq!(db.clear()?, 3);
        assert!(db.list(10, 0, None)?.is_empty());
        Ok(())
    }

    #[test]
    fn bodies_are_compressed_and_capped() -> Result<()> {
        let db = HistoryDb::open_in_memory()?;
        let mut big = entry("h_big", 1, "http://a");
        big.res_body = Some(vec![b'x'; MAX_BODY_BYTES + 1000]);
        db.insert(&big)?;
        let stored: Vec<u8> = db.conn().query_row(
            "SELECT res_body FROM history WHERE id = 'h_big'",
            [],
            |row| row.get(0),
        )?;
        assert!(
            stored.len() < 10_000,
            "zstd compressed: {} bytes",
            stored.len()
        );
        assert_eq!(
            db.get("h_big")?.unwrap().res_body.unwrap().len(),
            MAX_BODY_BYTES
        );
        Ok(())
    }

    #[test]
    fn source_check_constraint_matches_spec() -> Result<()> {
        let db = HistoryDb::open_in_memory()?;
        let bad = db.conn().execute(
            "INSERT INTO history (id, ts, source) VALUES ('x', 1, 'web')",
            [],
        );
        assert!(bad.is_err());
        Ok(())
    }
}
