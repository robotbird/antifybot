use crate::{now, protocol::*, ApiError, ApiResult};
use axum::http::StatusCode;
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Store(Arc<Mutex<Connection>>);

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY, sender TEXT NOT NULL, recipient TEXT NOT NULL,
                body TEXT NOT NULL, created_at INTEGER NOT NULL,
                acked INTEGER NOT NULL DEFAULT 0, receipt_acked INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS inbox ON messages(recipient, acked, created_at);
            CREATE INDEX IF NOT EXISTS receipts ON messages(sender, acked, receipt_acked);
            CREATE TABLE IF NOT EXISTS files (
                id TEXT PRIMARY KEY, owner TEXT NOT NULL, recipient TEXT NOT NULL,
                name TEXT NOT NULL, size INTEGER NOT NULL, sha256 TEXT NOT NULL DEFAULT '',
                expires_at INTEGER NOT NULL, ready INTEGER NOT NULL DEFAULT 0
            );
        ",
        )?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    pub(crate) async fn run<T, F>(&self, f: F) -> ApiResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> ApiResult<T> + Send + 'static,
    {
        let db = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = db.lock().map_err(ApiError::internal)?;
            f(&mut conn)
        })
        .await
        .map_err(ApiError::internal)?
    }

    pub async fn send(&self, from: String, request: SendMessage) -> ApiResult<()> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let body = serde_json::to_string(&request.body).map_err(ApiError::internal)?;
            let existing: Option<(String, String, String)> = tx.query_row(
                "SELECT sender,recipient,body FROM messages WHERE id=?1", [&request.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            ).optional()?;
            if let Some(old) = existing {
                return if old == (from, request.to, body) { Ok(()) }
                    else { Err(ApiError(StatusCode::CONFLICT, "message ID already used".into())) };
            }
            if let Body::File { file_id } = &request.body {
                let allowed: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM files WHERE id=?1 AND owner=?2 AND recipient=?3 AND ready=1 AND expires_at>?4)",
                    params![file_id, from, request.to, now()], |r| r.get(0),
                )?;
                if !allowed { return Err(ApiError::bad("file unavailable or not owned by sender")); }
            }
            let count: i64 = tx.query_row("SELECT COUNT(*) FROM messages WHERE sender=?1", [&from], |r| r.get(0))?;
            if count >= 10000 { return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "message quota reached".into())); }
            tx.execute("INSERT INTO messages(id,sender,recipient,body,created_at) VALUES(?1,?2,?3,?4,?5)",
                params![request.id, from, request.to, body, now()])?;
            tx.commit()?;
            Ok(())
        }).await
    }

    pub async fn events(&self, device: String) -> ApiResult<Vec<ServerEvent>> {
        self.run(move |db| {
            let mut result = Vec::new();
            let mut q = db.prepare("SELECT id,sender,recipient,body,created_at FROM messages WHERE recipient=?1 AND acked=0 ORDER BY rowid LIMIT 128")?;
            let rows = q.query_map([&device], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?, r.get::<_,String>(3)?, r.get::<_,i64>(4)?)))?;
            for row in rows {
                let (id, from, to, body, created_at) = row?;
                result.push(ServerEvent::Message { message: Message { id, from, to, created_at,
                    body: serde_json::from_str(&body).map_err(ApiError::internal)? } });
            }
            let mut q = db.prepare("SELECT id FROM messages WHERE sender=?1 AND acked=1 AND receipt_acked=0 ORDER BY rowid LIMIT 128")?;
            for id in q.query_map([&device], |r| r.get::<_,String>(0))? {
                result.push(ServerEvent::Delivered { id: id? });
            }
            Ok(result)
        }).await
    }

    pub async fn ack(&self, device: String, id: String) -> ApiResult<()> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let row: Option<(String, String, bool)> = tx
                .query_row(
                    "SELECT sender,recipient,acked FROM messages WHERE id=?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            match row {
                Some((_, recipient, _)) if device == recipient => {
                    tx.execute("UPDATE messages SET acked=1 WHERE id=?1", [&id])?;
                }
                Some((sender, _, true)) if device == sender => {
                    tx.execute("UPDATE messages SET receipt_acked=1 WHERE id=?1", [&id])?;
                }
                _ => {
                    return Err(ApiError(
                        StatusCode::NOT_FOUND,
                        "message not found or not acknowledgeable".into(),
                    ))
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    /// Only delete rows after their files were removed. On restart, discard incomplete uploads.
    pub async fn expired_files(&self, startup: bool) -> ApiResult<Vec<String>> {
        self.run(move |db| {
            let mut q =
                db.prepare("SELECT id FROM files WHERE expires_at<=?1 OR (?2 AND ready=0)")?;
            let ids = q
                .query_map(params![now(), startup], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ids)
        })
        .await
    }

    pub async fn remove_file(&self, id: String) -> ApiResult<()> {
        self.run(move |db| {
            db.execute("DELETE FROM files WHERE id=?1", [id])?;
            Ok(())
        })
        .await
    }

    pub async fn prune(&self, ttl: u64) -> ApiResult<()> {
        self.run(move |db| {
            db.execute(
                "DELETE FROM messages WHERE created_at<?1",
                [now() - ttl as i64],
            )?;
            Ok(())
        })
        .await
    }
}
