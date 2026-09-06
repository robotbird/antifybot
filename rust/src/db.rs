//! SQLite 持久化（~/.antifybot-rs/chat.db，WAL）
//! messages 是聊天消息的唯一真源（自增主键即消息 id，跨重启稳定）；
//! devices 是内存设备表的写穿副本（启动回填，离线设备因此跨重启可见）。
//! 所有查询都是毫秒级小操作，同步调用 + 短持锁即可，不必 spawn_blocking。
use crate::state::{ChatMsg, Device};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, Row};
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// 消息保留上限（启动时裁剪；行内只存元数据，5000 条约 1~2MB）
const MSG_KEEP: u64 = 5000;

pub struct Db(Arc<Mutex<Connection>>);

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("创建 {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("打开 {}", path.display()))?;
        // WAL：写穿设备的短事务与面板轮询的读互不阻塞；journal_mode 有返回值需 query_row
        let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
               id         INTEGER PRIMARY KEY AUTOINCREMENT,
               out        INTEGER NOT NULL,             -- 1 = 我方发出
               peer       TEXT NOT NULL,                -- 对端指纹（退化时为 IP 字符串）
               peer_alias TEXT NOT NULL DEFAULT '',
               kind       TEXT NOT NULL,                -- 'text' | 'file'
               text       TEXT NOT NULL DEFAULT '',
               name       TEXT NOT NULL DEFAULT '',
               size       INTEGER NOT NULL DEFAULT 0,
               at         INTEGER NOT NULL,             -- 毫秒时间戳
               file       TEXT NOT NULL DEFAULT '',     -- 已收文件名（相对下载目录）；出站为空
               status     TEXT NOT NULL DEFAULT '',     -- 出站: 'sending'|'ok'|'fail'；入站恒 ''
               src_path   TEXT NOT NULL DEFAULT ''      -- 出站文件源绝对路径（重试依据）
             );
             CREATE INDEX IF NOT EXISTS idx_messages_peer ON messages(peer, id);
             CREATE TABLE IF NOT EXISTS devices (
               fingerprint  TEXT PRIMARY KEY,
               alias        TEXT NOT NULL,
               ip           TEXT NOT NULL,
               port         INTEGER NOT NULL,
               https        INTEGER NOT NULL DEFAULT 1,
               device_model TEXT,
               device_type  TEXT,
               version      TEXT NOT NULL DEFAULT '',
               download     INTEGER NOT NULL DEFAULT 0,
               last_seen    INTEGER NOT NULL
             );",
        )?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    /// 插入消息，返回自增 id（出站图片回显缓存以此为主键）
    pub fn insert_msg(&self, m: &ChatMsg) -> Result<u64> {
        let c = self.0.lock().unwrap();
        c.execute(
            "INSERT INTO messages
               (out, peer, peer_alias, kind, text, name, size, at, file, status, src_path)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                m.out as i64, m.peer, m.peer_alias, m.kind, m.text, m.name,
                m.size as i64, m.at as i64, m.file, m.status, m.src_path,
            ],
        )?;
        Ok(c.last_insert_rowid() as u64)
    }

    pub fn set_msg_status(&self, id: u64, status: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("UPDATE messages SET status=?1 WHERE id=?2", params![status, id as i64])?;
        Ok(())
    }

    pub fn get_msg(&self, id: u64) -> Option<ChatMsg> {
        let c = self.0.lock().unwrap();
        let mut stmt = c
            .prepare(
                "SELECT id,out,peer,peer_alias,kind,text,name,size,at,file,status,src_path
                 FROM messages WHERE id=?1",
            )
            .ok()?;
        stmt.query_row(params![id as i64], msg_from_row).ok()
    }

    /// 最近 limit 条（时间正序返回）
    pub fn recent_msgs(&self, limit: u64) -> Vec<ChatMsg> {
        let Ok(c) = self.0.lock() else { return Vec::new() };
        let Ok(mut stmt) = c.prepare(
            "SELECT id,out,peer,peer_alias,kind,text,name,size,at,file,status,src_path
             FROM messages ORDER BY id DESC LIMIT ?1",
        ) else {
            return Vec::new();
        };
        let rows = stmt
            .query_map(params![limit as i64], msg_from_row)
            .map(|it| it.filter_map(|r| r.ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        let mut out = rows;
        out.reverse();
        out
    }

    /// 上次运行残留的 sending 全部落 fail（崩溃/断电时在途的消息让用户手动重试）；返回条数
    pub fn fail_pending_sending(&self) -> usize {
        let Ok(c) = self.0.lock() else { return 0 };
        c.execute("UPDATE messages SET status='fail' WHERE out=1 AND status='sending'", [])
            .unwrap_or(0)
    }

    /// 启动时裁剪历史（只留最近 MSG_KEEP 条）
    pub fn prune(&self) {
        let Ok(c) = self.0.lock() else { return };
        let _ = c.execute(
            "DELETE FROM messages WHERE id <= (SELECT MAX(id) - ?1 FROM messages)",
            params![MSG_KEEP as i64],
        );
    }

    pub fn upsert_device(&self, d: &Device) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute(
            "INSERT OR REPLACE INTO devices
               (fingerprint, alias, ip, port, https, device_model, device_type, version, download, last_seen)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                d.fingerprint, d.alias, d.ip.to_string(), d.port, d.https as i64,
                d.device_model, d.device_type, d.version, d.download as i64,
                d.last_seen as i64,
            ],
        )?;
        Ok(())
    }

    pub fn remove_device(&self, fp: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("DELETE FROM devices WHERE fingerprint=?1", params![fp])?;
        Ok(())
    }

    /// 启动回填：ip 解析失败的行跳过（不该发生，防御性处理）
    pub fn load_devices(&self) -> Vec<Device> {
        let Ok(c) = self.0.lock() else { return Vec::new() };
        let Ok(mut stmt) = c.prepare(
            "SELECT fingerprint, alias, ip, port, https, device_model, device_type,
                    version, download, last_seen FROM devices",
        ) else {
            return Vec::new();
        };
        let rows = stmt
            .query_map([], |r| {
                let ip: String = r.get("ip")?;
                let ip: IpAddr = ip.parse().map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                Ok(Device {
                    fingerprint: r.get("fingerprint")?,
                    alias: r.get("alias")?,
                    ip,
                    port: r.get::<_, i64>("port")? as u16,
                    https: r.get::<_, i64>("https")? != 0,
                    device_model: r.get("device_model")?,
                    device_type: r.get("device_type")?,
                    version: r.get("version")?,
                    download: r.get::<_, i64>("download")? != 0,
                    last_seen: r.get::<_, i64>("last_seen")? as u64,
                })
            })
            .map(|it| it.filter_map(|row| row.ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        rows
    }
}

fn msg_from_row(r: &Row) -> rusqlite::Result<ChatMsg> {
    Ok(ChatMsg {
        id: r.get::<_, i64>("id")? as u64,
        out: r.get::<_, i64>("out")? != 0,
        peer: r.get("peer")?,
        peer_alias: r.get("peer_alias")?,
        kind: r.get("kind")?,
        text: r.get("text")?,
        name: r.get("name")?,
        size: r.get::<_, i64>("size")? as u64,
        at: r.get::<_, i64>("at")? as u64,
        file: r.get("file")?,
        status: r.get("status")?,
        src_path: r.get("src_path")?,
    })
}
