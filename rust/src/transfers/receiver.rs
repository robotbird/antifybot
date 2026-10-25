use super::{Entry, Runtime, Task};
use crate::{shares, state::Shared};
use anyhow::{ensure, Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use rusqlite::params;
use serde::Deserialize;
use sha2::{Digest,Sha256};
use std::{path::{Path,PathBuf},sync::atomic::Ordering,time::Duration};
use tokio::io::{AsyncReadExt,AsyncSeekExt,AsyncWriteExt};
const CHUNK:u64=4*1024*1024;

#[derive(Debug)] struct FileRejected(String);
impl std::fmt::Display for FileRejected {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str(&self.0)}}
impl std::error::Error for FileRejected {}
fn reject(message:impl Into<String>)->anyhow::Error{FileRejected(message.into()).into()}

#[derive(Deserialize)] struct Page {status:String,error:String,files:u64,bytes:u64,entries:Vec<Entry>,cursor:i64}
async fn json<T:serde::de::DeserializeOwned>(response:reqwest::Response)->Result<T>{
    let status=response.status();let mut stream=response.bytes_stream();let mut bytes=Vec::new();
    while let Some(b)=stream.next().await {let b=b?;ensure!(bytes.len()+b.len()<=2*1024*1024,"清单响应超过单页大小限制");bytes.extend_from_slice(&b);}
    ensure!(status.is_success(),"对端拒绝请求（HTTP {}），请确认共享未撤销且双方版本支持目录任务",status);
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn run(state:&Shared,t:&mut Task,rt:&Runtime)->Result<()> {
    let store=&state.transfers.store;
    store.log(&t.id,"info","开始/恢复任务；已完成文件和持久断点保留").await?;
    let _awake=match super::power::Guard::acquire() {
        Ok(g)=>Some(g),
        Err(e)=>{store.log(&t.id,"warn",&format!("防休眠不可用：{e}；请保持本机和共享方唤醒")).await?;None}
    };
    let (client,base)=rt.wait(shares::remote(state,&t.peer,&t.token)).await?;
    if !t.scanned {
        t.state="scanning".into();store.save(t).await?;
        loop {
            rt.check()?;
            let p:Page=rt.wait(async {json(client.get(format!("{base}/api/antify/v2/share/index"))
                .query(&[("token",t.token.clone()),("manifest",t.id.clone()),("after",t.cursor.to_string())])
                .timeout(Duration::from_secs(30)).send().await?).await}).await?;
            ensure!(p.status!="failed","目录整理失败：{}",p.error);
            ensure!(p.entries.len()<=200 && p.cursor>=t.cursor,"无效清单页");
            let empty=p.entries.is_empty();let mut previous=t.cursor;
            for e in &p.entries {
                ensure!(e.id>previous && e.id<=p.cursor,"清单游标不连续");previous=e.id;
                ensure!(!e.path.is_empty() && e.path.len()<=4096,"无效文件路径");shares::parts(&e.path)?;
                ensure!(e.size<=i64::MAX as u64,"文件过大");
            }
            ensure!(p.cursor==previous,"无效清单游标");
            t.cursor=p.cursor;t.files=p.files;t.bytes=p.bytes;
            t.scanned=p.status=="ready" && empty;
            let task=t.clone();
            store.run(move |c| {
                let tx=c.transaction()?;
                for e in p.entries {
                    tx.execute("INSERT OR IGNORE INTO entries(task,id,path,directory,size,version) VALUES (?,?,?,?,?,?)",params![task.id,e.id,e.path,e.directory,e.size as i64,e.version])?;
                }
                super::store::save(&tx,&task)?;tx.commit()?;Ok(())
            }).await?;
            if t.scanned {store.log(&t.id,"info",&format!("清单完成：{} 个文件，{} 字节",t.files,t.bytes)).await?;break;}
            if empty {rt.wait(async {tokio::time::sleep(Duration::from_secs(1)).await;Ok(())}).await?;}
        }
    }
    // Rebuild aggregate counters once per run from transactional entry checkpoints.
    // A failed task-state write must never make retries double-count a block.
    let id=t.id.clone();
    let (files,bytes,received,completed,failed)=store.run(move |c| {
        Ok(c.query_row("SELECT COUNT(CASE WHEN directory=0 THEN 1 END),COALESCE(SUM(size),0),COALESCE(SUM(offset),0),COUNT(CASE WHEN directory=0 AND status='done' THEN 1 END),COUNT(CASE WHEN status='failed' THEN 1 END) FROM entries WHERE task=?",[id],|r|Ok((r.get::<_,i64>(0)? as u64,r.get::<_,i64>(1)? as u64,r.get::<_,i64>(2)? as u64,r.get::<_,i64>(3)? as u64,r.get::<_,i64>(4)? as u64)))?)
    }).await?;
    t.files=files;t.bytes=bytes;t.received=received;t.completed=completed;t.failed=failed;
    rt.check()?;
    let parent=t.destination.parent().context("目标路径无效")?;
    ensure!(tokio::fs::canonicalize(parent).await?==parent,"保存目录已被移动或替换");
    let name=t.destination.file_name().and_then(|n|n.to_str()).context("目标名称无效")?;
    shares::safe_dir(parent,&[name]).await?;
    let stage=shares::safe_dir(parent,&[&format!(".antify-transfer-{}",t.id)]).await?;
    let remaining=t.bytes.saturating_sub(t.received);
    check_space(&stage,remaining).await?;
    t.state="receiving".into();store.save(t).await?;
    while let Some(mut e)=store.next(&t.id).await? {
        rt.check()?;t.current=e.path.clone();t.state="receiving".into();store.save(t).await?;
        let result=if e.directory {create_directory(&t.destination,&e.path).await.map(|_|())}
            else {receive(state,t,&mut e,rt,&client,&base,&stage).await};
        match result {
            Ok(())=>{
                e.status="done".into();if !e.directory {t.completed+=1;}
                store.checkpoint(t,&e).await?;
                if !e.directory {store.log(&t.id,"info",&format!("完成 {} · {} 字节 · SHA-256 通过",e.path,e.size)).await?;}
            },
            Err(error) if error.downcast_ref::<FileRejected>().is_some()=>{
                e.status="failed".into();e.error=super::redact(&format!("{error:#}"),&t.token);t.failed+=1;
                store.checkpoint(t,&e).await?;
                store.log(&t.id,"error",&format!("{}：{}",e.path,e.error)).await?;
            },
            Err(error)=>return Err(error), // Network/disk errors pause the whole task, never discard partials.
        }
    }
    t.state=if t.failed==0{"done"}else{"partial"}.into();t.current.clear();t.finished=crate::state::now_ms();
    store.save(t).await?;
    store.log(&t.id,"info",&format!("任务结束：完成 {}，失败 {}，已校验 {} 字节",t.completed,t.failed,t.received)).await?;
    if t.failed==0 {let _=tokio::fs::remove_dir(&stage).await;}
    Ok(())
}
async fn create_directory(root:&Path,path:&str)->Result<PathBuf>{shares::safe_dir(root,&shares::parts(path)?).await}

async fn receive(state:&Shared,t:&mut Task,e:&mut Entry,rt:&Runtime,client:&Client,base:&str,stage:&Path)->Result<()> {
    // This check records a fixed plan; changed sources require a new offer, not silent version mixing.
    let m:shares::Manifest=rt.wait(async {json(client.get(format!("{base}/api/antify/v1/share/manifest"))
        .query(&[("token",&t.token),("path",&e.path)]).timeout(Duration::from_secs(3600)).send().await?).await}).await?;
    if m.size!=e.size || m.version!=e.version {return Err(reject("源文件已改变，请重新发送文件夹建立新计划"))}
    ensure!(m.sha256.len()==64 && m.sha256.bytes().all(|b|b.is_ascii_hexdigit()),"无效摘要");
    if !e.sha.is_empty() && e.sha!=m.sha256 {return Err(reject("源文件内容已改变，旧断点不能继续"))}
    e.sha=m.sha256;
    let parts=shares::parts(&e.path)?;
    let dest_dir=shares::safe_dir(&t.destination,&parts[..parts.len()-1]).await?;
    let dest=dest_dir.join(parts.last().unwrap());
    // Reconcile publication-before-checkpoint crashes, and never overwrite unrelated data.
    if let Ok(meta)=tokio::fs::symlink_metadata(&dest).await {
        if meta.is_file() && !meta.file_type().is_symlink() && meta.len()==e.size && hash(&dest,rt).await?==e.sha {
            t.received+=e.size.saturating_sub(e.offset);e.offset=e.size;return Ok(())
        }
        return Err(reject("目标同名文件内容不同，已保留原文件；请更换目标或移走冲突文件后重试"))
    }
    let part=stage.join(format!("{}.part",e.id));
    if let Ok(m)=tokio::fs::symlink_metadata(&part).await {ensure!(m.is_file() && !m.file_type().is_symlink(),"暂存路径不安全");}
    let mut f=tokio::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&part).await?;
    let len=f.metadata().await?.len();
    if len<e.offset {t.received=t.received.saturating_sub(e.offset);e.offset=0;}
    // Checkpoint only follows sync_data; uncommitted tail is never trusted on restart.
    f.set_len(e.offset).await?;f.seek(std::io::SeekFrom::Start(e.offset)).await?;
    state.transfers.store.checkpoint(t,e).await?;
    let mut last_log=std::time::Instant::now();
    let mut last_network=rt.network.load(Ordering::Relaxed);
    while e.offset<e.size {
        rt.check()?;
        let mut chunk=None;
        for attempt in 0..3 {
            let result=rt.wait(async {
                let response=client.get(format!("{base}/api/antify/v1/share/file"))
                    .query(&[("token",t.token.clone()),("path",e.path.clone()),("version",e.version.clone()),("offset",e.offset.to_string())])
                    .timeout(Duration::from_secs(120)).send().await?;
                ensure!(response.status()==reqwest::StatusCode::PARTIAL_CONTENT,"文件读取被拒绝（共享撤销或源文件改变）");
                let digest=response.headers().get("x-antify-sha256").context("缺少块摘要")?.to_str()?.to_owned();
                let expected=CHUNK.min(e.size-e.offset) as usize;
                let mut stream=response.bytes_stream();let mut bytes=Vec::with_capacity(expected);
                while let Some(b)=stream.next().await {let b=b?;ensure!(bytes.len()+b.len()<=expected,"块大小异常");rt.network.fetch_add(b.len() as u64,Ordering::Relaxed);bytes.extend_from_slice(&b);}
                ensure!(bytes.len()==expected && hex::encode(Sha256::digest(&bytes))==digest,"块校验失败");
                Ok(bytes)
            }).await;
            match result {Ok(bytes)=>{chunk=Some(bytes);break},Err(error)=>{
                rt.check()?;
                state.transfers.store.log(&t.id,"warn",&format!("{} 块重试 {}：{}",e.path,attempt+1,super::redact(&error.to_string(),&t.token))).await?;
                if attempt==2 {return Err(error)}
                rt.wait(async {tokio::time::sleep(Duration::from_secs(1<<attempt)).await;Ok(())}).await?;
            }}
        }
        let bytes=chunk.context("无法读取数据块")?;
        check_space(stage,bytes.len() as u64).await?;
        f.write_all(&bytes).await?;f.sync_data().await?;
        e.offset+=bytes.len() as u64;t.received+=bytes.len() as u64;
        state.transfers.store.checkpoint(t,e).await?;
        if last_log.elapsed()>=Duration::from_secs(10) {
            let network=rt.network.load(Ordering::Relaxed);
            let speed=network.saturating_sub(last_network) as f64/last_log.elapsed().as_secs_f64()/1_000_000.0;
            state.transfers.store.log(&t.id,"info",&format!("进度 {}/{} 字节，{:.1} MB/s；当前 {}",t.received,t.bytes,speed,e.path)).await?;
            last_log=std::time::Instant::now();last_network=network;
        }
    }
    f.sync_all().await?;drop(f);
    t.state="verifying".into();state.transfers.store.save(t).await?;
    if hash(&part,rt).await?!=e.sha {
        // Keep original target untouched. Reset only this task's verified checkpoint.
        t.received=t.received.saturating_sub(e.offset);e.offset=0;
        state.transfers.store.checkpoint(t,e).await?;
        tokio::fs::remove_file(&part).await?;
        return Err(reject("整文件校验失败，已清除损坏断点，可重试"))
    }
    rt.check()?;
    // Same-volume atomic no-clobber publication. Reconciliation above handles a crash after this.
    tokio::fs::hard_link(&part,&dest).await.context("目标已存在或文件系统不支持安全发布，未覆盖目标")?;
    tokio::fs::remove_file(&part).await?;
    t.state="receiving".into();Ok(())
}
async fn check_space(path:&Path,needed:u64)->Result<()> {
    let path=path.to_owned();
    let free=tokio::task::spawn_blocking(move || fs2::available_space(path)).await??;
    ensure!(free>=needed.saturating_add(16*1024*1024),"保存磁盘空间不足（需另留 16 MiB 余量），请清理空间后继续");
    Ok(())
}
async fn hash(path:&Path,rt:&Runtime)->Result<String>{
    let mut f=tokio::fs::File::open(path).await?;let mut buf=vec![0;256*1024];let mut h=Sha256::new();
    loop {rt.check()?;let n=f.read(&mut buf).await?;if n==0{break}h.update(&buf[..n]);}
    Ok(hex::encode(h.finalize()))
}
