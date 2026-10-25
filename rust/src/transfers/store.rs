//! Bounded, transactional transfer storage. All calls run on blocking threads.
use super::{Entry, Task};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{io::Write, path::{Path, PathBuf}, time::Duration};

#[derive(Clone)]
pub struct Store { pub root: PathBuf }
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let s = Self { root: root.to_owned() };
        let c = s.connect()?;
        c.pragma_update(None, "journal_mode", "WAL")?;
        c.execute_batch("CREATE TABLE IF NOT EXISTS tasks (
          id TEXT PRIMARY KEY, message_id INTEGER UNIQUE NOT NULL, data TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS entries (
          task TEXT NOT NULL, id INTEGER NOT NULL, path TEXT NOT NULL, directory INTEGER NOT NULL,
          size INTEGER NOT NULL, version TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending',
          offset INTEGER NOT NULL DEFAULT 0, sha TEXT NOT NULL DEFAULT '', error TEXT NOT NULL DEFAULT '',
          PRIMARY KEY(task,id), UNIQUE(task,path));
          CREATE INDEX IF NOT EXISTS entry_queue ON entries(task,status,id);
          CREATE TABLE IF NOT EXISTS indexes (id TEXT PRIMARY KEY, token TEXT NOT NULL,
          status TEXT NOT NULL DEFAULT 'scanning', error TEXT NOT NULL DEFAULT '',
          files INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0);
          CREATE TABLE IF NOT EXISTS index_dirs (manifest TEXT NOT NULL, path TEXT NOT NULL,
          done INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(manifest,path));
          CREATE INDEX IF NOT EXISTS dir_queue ON index_dirs(manifest,done);
          CREATE TABLE IF NOT EXISTS index_entries (id INTEGER PRIMARY KEY AUTOINCREMENT,
          manifest TEXT NOT NULL, path TEXT NOT NULL, directory INTEGER NOT NULL,
          size INTEGER NOT NULL, version TEXT NOT NULL, UNIQUE(manifest,path));
          CREATE INDEX IF NOT EXISTS index_page ON index_entries(manifest,id);
          CREATE TABLE IF NOT EXISTS logs (id INTEGER PRIMARY KEY AUTOINCREMENT,
          task TEXT NOT NULL, at INTEGER NOT NULL, level TEXT NOT NULL, message TEXT NOT NULL);
          CREATE INDEX IF NOT EXISTS task_logs ON logs(task,id);")?;
        // Crashed workers never auto-resume. Their last durable block is retained.
        let mut stmt = c.prepare("SELECT data FROM tasks")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for row in rows {
            let mut t: Task = serde_json::from_str(&row?)?;
            if matches!(t.state.as_str(), "queued"|"scanning"|"receiving"|"verifying"|"connecting") {
                t.state = "paused".into(); t.error = "上次运行中断，点击继续恢复".into();
                save(&c, &t)?;
            }
        }
        Ok(s)
    }
    pub fn connect(&self) -> Result<Connection> {
        let c = Connection::open(self.root.join("transfers.sqlite"))?;
        c.busy_timeout(Duration::from_secs(15))?;
        c.pragma_update(None, "synchronous", "FULL")?;
        Ok(c)
    }
    pub async fn run<T, F>(&self, f: F) -> Result<T>
    where T: Send + 'static, F: FnOnce(&mut Connection) -> Result<T> + Send + 'static {
        let s = self.clone();
        tokio::task::spawn_blocking(move || f(&mut s.connect()?)).await?
    }
    pub async fn task(&self, id: &str) -> Result<Task> {
        let id = id.to_owned(); self.run(move |c| load(c, &id)).await
    }
    pub async fn save(&self, task: &Task) -> Result<()> {
        let t = task.clone(); self.run(move |c| save(c, &t)).await
    }
    pub async fn log(&self, id: &str, level: &str, message: &str) -> Result<()> {
        let (id, level, message) = (id.to_owned(), level.to_owned(), message.to_owned());
        let root = self.root.clone();
        self.run(move |c| {
            let at = crate::state::now_ms();
            c.execute("INSERT INTO logs(task,at,level,message) VALUES (?,?,?,?)", params![id,at as i64,level,message])?;
            let event = c.last_insert_rowid();
            // SQLite is the durable event source. JSONL is an append-only companion, never a checkpoint.
            let dir = root.join("logs").join(&id);
            std::fs::create_dir_all(&dir)?;
            let active = dir.join("current.jsonl");
            if std::fs::metadata(&active).map(|m| m.len() >= 8*1024*1024).unwrap_or(false) {
                std::fs::rename(&active, dir.join(format!("before-{event}.jsonl")))?;
            }
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(active)?;
            writeln!(f, "{}", json!({"id":event,"at":at,"level":level,"message":message}))?;
            f.sync_data()?;
            Ok(())
        }).await
    }
    pub async fn checkpoint(&self, task: &Task, entry: &Entry) -> Result<()> {
        let (t, e) = (task.clone(), entry.clone());
        self.run(move |c| {
            let tx = c.transaction()?;
            tx.execute("UPDATE entries SET status=?,offset=?,sha=?,error=? WHERE task=? AND id=?",
                params![e.status,e.offset as i64,e.sha,e.error,t.id,e.id])?;
            save(&tx,&t)?; tx.commit()?; Ok(())
        }).await
    }
    pub async fn next(&self, id: &str) -> Result<Option<Entry>> {
        let id = id.to_owned();
        self.run(move |c| Ok(c.query_row("SELECT id,path,directory,size,version,status,offset,sha,error FROM entries WHERE task=? AND status='pending' ORDER BY id LIMIT 1", [id], entry_row).optional()?)).await
    }
    pub async fn logs(&self, id: String, after: i64) -> Result<Vec<Value>> {
        self.run(move |c| {
            let mut q = c.prepare("SELECT id,at,level,message FROM logs WHERE task=? AND id>? ORDER BY id LIMIT 200")?;
            let rows = q.query_map(params![id,after], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"at":r.get::<_,i64>(1)?,"level":r.get::<_,String>(2)?,"message":r.get::<_,String>(3)?})))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await
    }
}
pub fn save(c: &Connection, t: &Task) -> Result<()> {
    c.execute("UPDATE tasks SET data=? WHERE id=?", params![serde_json::to_string(t)?,t.id])?;
    Ok(())
}
pub fn load(c: &Connection, id: &str) -> Result<Task> {
    let data: String = c.query_row("SELECT data FROM tasks WHERE id=?",[id],|r|r.get(0)).optional()?.context("任务不存在")?;
    Ok(serde_json::from_str(&data)?)
}
fn entry_row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry { id:r.get(0)?, path:r.get(1)?, directory:r.get(2)?, size:r.get::<_,i64>(3)? as u64, version:r.get(4)?,status:r.get(5)?,offset:r.get::<_,i64>(6)? as u64,sha:r.get(7)?,error:r.get(8)? })
}
