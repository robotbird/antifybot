//! Incremental, disk-backed manifests. Page size is bounded, total entry count isn't.
use super::{store::Store, Entry};
use crate::{shares, state::Shared};
use anyhow::{ensure, Context, Result};
use axum::{extract::{Query, State}, response::Response};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub struct Page { pub token: String, pub manifest: String, #[serde(default)] pub after: i64 }

pub async fn page(State(state): State<Shared>, Query(q): Query<Page>) -> Response {
    shares::reply(async {
        shares::valid_token(&q.manifest)?;
        ensure!(q.after >= 0, "无效游标");
        let share = shares::load(&state, &q.token).await?; // Revocation checked on every page.
        let store = state.transfers.store.clone();
        let (id, token) = (q.manifest.clone(),q.token.clone());
        let status = store.run(move |c| {
            let tx = c.transaction()?;
            tx.execute("INSERT OR IGNORE INTO indexes(id,token) VALUES (?,?)", params![id,token])?;
            let (owner,status):(String,String) = tx.query_row("SELECT token,status FROM indexes WHERE id=?",[&id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            ensure!(owner == token, "清单授权不匹配");
            tx.execute("INSERT OR IGNORE INTO index_dirs(manifest,path) VALUES (?,'')",[&id])?;
            tx.commit()?; Ok(status)
        }).await?;
        if status == "scanning" {
            let mut active = state.transfers.scans.lock().await;
            if !active.contains(&q.manifest) && active.len() < 2 {
                active.insert(q.manifest.clone());
                let st = state.clone(); let id = q.manifest.clone();
                let root = share.root.clone(); let token = q.token.clone();
                tokio::spawn(async move {
                    let s = st.transfers.store.clone();
                    let sid = id.clone();
                    let credential = st.identity.cfg_dir.join("shares").join(format!("{token}.json"));
                    let result = tokio::task::spawn_blocking(move || scan(&s,&sid,&root,&credential)).await;
                    let error = match result { Ok(Ok(())) => None, Ok(Err(e)) => Some(format!("{e:#}")), Err(e) => Some(e.to_string()) };
                    if let Some(error) = error {
                        let sid = id.clone();
                        let _ = st.transfers.store.run(move |c| { c.execute("UPDATE indexes SET status='failed',error=? WHERE id=?",params![error,sid])?; Ok(()) }).await;
                    }
                    st.transfers.scans.lock().await.remove(&id);
                });
            }
        }
        let id = q.manifest; let after = q.after;
        let mut v = store.run(move |c| {
            let (status,error,files,bytes):(String,String,u64,u64) = c.query_row("SELECT status,error,files,bytes FROM indexes WHERE id=?",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get::<_,i64>(2)? as u64,r.get::<_,i64>(3)? as u64)))?;
            let mut stmt = c.prepare("SELECT id,path,directory,size,version FROM index_entries WHERE manifest=? AND id>? ORDER BY id LIMIT 200")?;
            let rows = stmt.query_map(params![id,after],|r| Ok(Entry { id:r.get(0)?,path:r.get(1)?,directory:r.get(2)?,size:r.get::<_,i64>(3)? as u64,version:r.get(4)?,..Default::default() }))?;
            let entries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            let next = entries.last().map(|e|e.id).unwrap_or(after);
            Ok(json!({"status":status,"error":error,"files":files,"bytes":bytes,"entries":entries,"cursor":next}))
        }).await?;
        v["name"] = json!(share.name);
        Ok(v)
    }.await)
}

fn scan(store: &Store, id: &str, root: &std::path::Path, credential: &std::path::Path) -> Result<()> {
    let mut c = store.connect()?;
    loop {
        ensure!(credential.is_file(), "共享已撤销");
        ensure!(std::fs::canonicalize(root)? == root, "共享目录已改变");
        let path: Option<String> = c.query_row("SELECT path FROM index_dirs WHERE manifest=? AND done=0 LIMIT 1",[id],|r|r.get(0)).optional()?;
        let Some(path) = path else { break };
        let mut dir = root.to_owned();
        for p in shares::parts(&path)? {
            dir.push(p); ensure!(!std::fs::symlink_metadata(&dir)?.file_type().is_symlink(), "目录含符号链接");
        }
        let mut batch = Vec::with_capacity(128);
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().into_string().map_err(|_|anyhow::anyhow!("目录包含非 UTF-8 文件名"))?;
            shares::parts(&name).context("目录包含不支持的跨平台文件名")?;
            let m = std::fs::symlink_metadata(entry.path())?;
            // Never silently produce a successful but incomplete migration.
            ensure!(m.is_file() || m.is_dir(), "目录含符号链接或特殊文件：{name}；请排除此项后重新发送");
            let relative = if path.is_empty() {name} else {format!("{path}/{name}")};
            batch.push(Entry { path:relative,directory:m.is_dir(),size:if m.is_dir(){0}else{m.len()},version:shares::version(&m)?,..Default::default() });
            if batch.len() == 128 { ensure!(credential.is_file(),"共享已撤销"); insert_batch(&mut c,id,&batch)?; batch.clear(); }
        }
        insert_batch(&mut c,id,&batch)?;
        c.execute("UPDATE index_dirs SET done=1 WHERE manifest=? AND path=?",params![id,path])?;
    }
    c.execute("UPDATE indexes SET status='ready' WHERE id=?",[id])?;
    Ok(())
}
fn insert_batch(c: &mut rusqlite::Connection, id: &str, entries: &[Entry]) -> Result<()> {
    let tx = c.transaction()?;
    let (mut files,mut bytes) = (0u64,0u64);
    for e in entries {
        let n = tx.execute("INSERT OR IGNORE INTO index_entries(manifest,path,directory,size,version) VALUES (?,?,?,?,?)",params![id,e.path,e.directory,i64::try_from(e.size)?,e.version])?;
        if e.directory { tx.execute("INSERT OR IGNORE INTO index_dirs(manifest,path) VALUES (?,?)",params![id,e.path])?; }
        else if n > 0 { files+=1; bytes=bytes.checked_add(e.size).context("目录总量溢出")?; }
    }
    tx.execute("UPDATE indexes SET files=files+?,bytes=bytes+? WHERE id=?",params![i64::try_from(files)?,i64::try_from(bytes)?,id])?;
    tx.commit()?; Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_have_no_total_count_limit_and_replay_is_idempotent() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let s = Store::open(&root).unwrap(); let mut c = s.connect().unwrap();
        c.execute("INSERT INTO indexes(id,token) VALUES ('test','token')",[]).unwrap();
        for batch in 0..81 {
            let entries = (0..128).map(|n| Entry{path:format!("{}",batch*128+n),size:1,..Default::default()}).collect::<Vec<_>>();
            insert_batch(&mut c,"test",&entries).unwrap(); insert_batch(&mut c,"test",&entries).unwrap();
        }
        let counts:(i64,i64) = c.query_row("SELECT files,bytes FROM indexes",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(counts,(10368,10368));
        drop(c); std::fs::remove_dir_all(root).unwrap();
    }
}
