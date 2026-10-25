//! Conversation-owned, durable folder receive tasks. No file-count limit.
//! This first protocol revision retains the existing explicitly authorized bearer share.
//! It does not claim authenticated one-shot offers or sender delivery receipts.
pub mod index;
mod power;
mod receiver;
mod store;
use crate::{shares, state::Shared};
use anyhow::{ensure, Context, Result};
use axum::{body::Body, extract::{Path, Query, State}, http::{header, StatusCode}, response::{IntoResponse, Response}, Json};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::{HashMap,HashSet,VecDeque}, path::PathBuf, sync::{Arc,atomic::{AtomicU8,AtomicU64,Ordering}}, time::{Duration,Instant}};
use tokio::sync::Mutex;

#[derive(Clone,Serialize,Deserialize)]
pub struct Task {
    pub id:String, pub message_id:u64, pub peer:String, pub token:String, pub name:String,
    pub destination:PathBuf, pub state:String, pub error:String, pub cursor:i64,
    pub scanned:bool, pub files:u64, pub bytes:u64, pub completed:u64, pub received:u64,
    pub failed:u64, pub current:String, pub created:u64,
    #[serde(default)] pub finished:u64,
}
#[derive(Clone,Default,Serialize,Deserialize)]
pub struct Entry {
    pub id:i64, pub path:String, pub directory:bool, pub size:u64, pub version:String,
    #[serde(default)] pub status:String, #[serde(default)] pub offset:u64,
    #[serde(default)] pub sha:String, #[serde(default)] pub error:String,
}
pub struct Runtime {
    pub control:AtomicU8, pub network:AtomicU64,
    samples:Mutex<VecDeque<(Instant,u64,u64)>>,
}
impl Runtime {
    fn new(received:u64) -> Self {Self{control:AtomicU8::new(0),network:AtomicU64::new(0),samples:Mutex::new(VecDeque::from([(Instant::now(),0,received)]))}}
    pub fn check(&self) -> Result<()> {ensure!(self.control.load(Ordering::Relaxed)==0,"任务已暂停或取消");Ok(())}
    pub async fn wait<T>(&self, f: impl std::future::Future<Output=Result<T>>) -> Result<T> {
        self.check()?;
        tokio::select! { r=f=>r, _=async { while self.control.load(Ordering::Relaxed)==0 {tokio::time::sleep(Duration::from_millis(100)).await;} }=>{anyhow::bail!("任务已暂停或取消")} }
    }
    async fn speed(&self,received:u64) -> (f64,f64) {
        let mut s = self.samples.lock().await; let now = Instant::now();
        let bytes=self.network.load(Ordering::Relaxed);
        s.push_back((now,bytes,received));
        while s.len()>2 && now.duration_since(s[1].0)>Duration::from_secs(5) {s.pop_front();}
        let (at,start,verified)=s[0];
        let seconds=now.duration_since(at).as_secs_f64().max(0.1);
        (bytes.saturating_sub(start) as f64/seconds,received.saturating_sub(verified) as f64/seconds)
    }
}
pub struct Manager {
    pub store:store::Store,
    active:Mutex<HashMap<String,Arc<Runtime>>>,
    scans:Mutex<HashSet<String>>,
}
impl Manager {
    pub fn open(root:&std::path::Path)->Result<Self>{Ok(Self{store:store::Store::open(root)?,active:Mutex::new(HashMap::new()),scans:Mutex::new(HashSet::new())})}
}

#[derive(Deserialize)]
pub struct Create { message_id:u64, directory:Option<String> }
#[derive(Deserialize)]
pub struct List { peer:String, #[serde(default)] before:u64 }
#[derive(Deserialize)]
pub struct LogQuery { #[serde(default)] after:i64 }

fn address(text:&str)->Result<(String,String)> {
    let start=text.find("antify-share://").context("消息不是文件夹邀请")?+"antify-share://".len();
    let raw=text[start..].split_whitespace().next().context("地址为空")?;
    let (peer,token)=raw.split_once('/').context("无效共享地址")?;
    ensure!(peer.len()==64 && peer.bytes().all(|c|c.is_ascii_hexdigit()),"无效设备指纹");
    shares::valid_token(token)?; Ok((peer.to_uppercase(),token.into()))
}

pub async fn create(State(state):State<Shared>,Json(body):Json<Create>)->Response {
    shares::reply(async {
        let m=state.db.get_msg(body.message_id).context("会话消息不存在")?;
        ensure!(!m.out,"只能接收对方发送的文件夹");
        let (peer,token)=address(&m.text)?;
        ensure!(m.peer.eq_ignore_ascii_case(&peer),"共享地址与消息发送方不一致");
        let id=uuid::Uuid::new_v4().simple().to_string();
        let chosen=body.directory.map(PathBuf::from).unwrap_or(state.download_dir.read().await.clone());
        let root=tokio::fs::canonicalize(chosen).await?;
        ensure!(tokio::fs::metadata(&root).await?.is_dir(),"保存位置不是文件夹");
        let raw=m.text.lines().next().unwrap_or("文件夹").trim_start_matches('📂').trim();
        let name=if !raw.is_empty() && raw.len()<120 && shares::parts(raw).is_ok() && !raw.contains('/') {raw.to_owned()} else {"文件夹".into()};
        let destination=root.join(format!("{}-{}",name,&id[..8]));
        let task=Task{id:id.clone(),message_id:m.id,peer,token,name,destination,state:"queued".into(),error:String::new(),cursor:0,scanned:false,files:0,bytes:0,completed:0,received:0,failed:0,current:String::new(),created:crate::state::now_ms(),finished:0};
        let t=state.transfers.store.run(move |c| {
            let existing:Option<String>=c.query_row("SELECT data FROM tasks WHERE message_id=?",[task.message_id as i64],|r|r.get(0)).optional()?;
            if let Some(data)=existing {return Ok(serde_json::from_str::<Task>(&data)?)}
            c.execute("INSERT OR IGNORE INTO tasks(id,message_id,data) VALUES (?,?,?)",params![task.id,task.message_id as i64,serde_json::to_string(&task)?])?;
            let data:String=c.query_row("SELECT data FROM tasks WHERE message_id=?",[task.message_id as i64],|r|r.get(0))?;
            Ok(serde_json::from_str::<Task>(&data)?)
        }).await?;
        if t.state=="queued" {launch(&state,&t.id).await?;}
        Ok(json!({"task":t}))
    }.await)
}

async fn launch(state:&Shared,id:&str)->Result<()> {
    let mut active=state.transfers.active.lock().await;
    if active.contains_key(id){return Ok(())}
    ensure!(active.len()<2,"已有两个任务运行，请暂停其中一个后继续");
    let mut t=state.transfers.store.task(id).await?;
    ensure!(!matches!(t.state.as_str(),"done"|"cancelled"),"任务已结束");
    t.state="connecting".into();t.error.clear();
    state.transfers.store.save(&t).await?;
    let rt=Arc::new(Runtime::new(t.received));active.insert(id.into(),rt.clone());
    let st=state.clone();let id=id.to_owned();
    tokio::spawn(async move {
        let result=receiver::run(&st,&mut t,&rt).await;
        if let Err(e)=result {
            let code=rt.control.load(Ordering::Relaxed);
            t.state=match code{1=>"paused",2=>"cancelled",_=>"interrupted"}.into();
            // Errors generated by us are redacted before logging: reqwest includes full query URLs.
            t.error=redact(&format!("{e:#}"),&t.token);
            if let Err(db)=st.transfers.store.save(&t).await {eprintln!("[任务] 无法持久化状态: {db:#}");}
            if let Err(log)=st.transfers.store.log(&id,"warn",&t.error).await {eprintln!("[任务] 无法写日志: {log:#}");}
        }
        st.transfers.active.lock().await.remove(&id);
    });
    Ok(())
}
pub(super) fn redact(text:&str,token:&str)->String {text.replace(token,"[redacted]")}

pub async fn list(State(state):State<Shared>,Query(q):Query<List>)->Response {
    shares::reply(async {
        let tasks:Vec<Task>=state.transfers.store.run(move |c| {
            let mut s=c.prepare("SELECT data FROM tasks WHERE json_extract(data,'$.peer')=? AND (?=0 OR message_id<?) ORDER BY message_id DESC LIMIT 100")?;
            let rows=s.query_map(params![q.peer,q.before as i64,q.before as i64],|r|r.get::<_,String>(0))?;
            rows.map(|r|Ok(serde_json::from_str(&r?)?)).collect()
        }).await?;
        let active=state.transfers.active.lock().await;
        let mut values=Vec::new();
        for t in tasks {
            let (speed,effective)=if t.state=="receiving" {if let Some(r)=active.get(&t.id){r.speed(t.received).await}else{(0.0,0.0)}}else{(0.0,0.0)};
            let mut v=serde_json::to_value(&t)?;v["speed"]=json!(speed);
            v["eta_seconds"]=if t.scanned && effective>0.0 {json!(t.bytes.saturating_sub(t.received) as f64/effective)}else{json!(null)};
            // Accurate ETA requires effective completion speed, not retransmission traffic.
            v["percent"]=if t.scanned && t.bytes>0 {json!((t.received as f64/t.bytes as f64*100.0).min(100.0))}else{json!(null)};
            values.push(v);
        }
        Ok(json!({"tasks":values}))
    }.await)
}
pub async fn control(State(state):State<Shared>,Path((id,action)):Path<(String,String)>)->Response {
    shares::reply(async {
        shares::valid_token(&id)?;
        let mut t=state.transfers.store.task(&id).await?;
        match action.as_str() {
            "open"=>{
                let dest=tokio::fs::canonicalize(&t.destination).await?;
                ensure!(dest==t.destination,"目标目录已移动或被替换");
                tokio::task::spawn_blocking(move || -> Result<()> {
                    #[cfg(target_os="macos")] let mut cmd=std::process::Command::new("open");
                    #[cfg(target_os="windows")] let mut cmd=std::process::Command::new("explorer");
                    #[cfg(not(any(target_os="macos",target_os="windows")))] let mut cmd=std::process::Command::new("xdg-open");
                    cmd.arg(dest).spawn()?;Ok(())
                }).await??;
            },
            "pause"|"cancel"=>{
                if let Some(rt)=state.transfers.active.lock().await.get(&id){rt.control.store(if action=="pause"{1}else{2},Ordering::Relaxed);}
                else if !matches!(t.state.as_str(),"done"|"cancelled") {t.state=if action=="pause"{"paused"}else{"cancelled"}.into();state.transfers.store.save(&t).await?;}
            },
            "resume"|"retry"=>{
                ensure!(!state.transfers.active.lock().await.contains_key(&id),"正在停止，请稍后继续");
                if action=="retry" {
                    let tid=id.clone();state.transfers.store.run(move |c|{c.execute("UPDATE entries SET status='pending',error='' WHERE task=? AND status='failed'",[tid])?;Ok(())}).await?;
                    t.failed=0;state.transfers.store.save(&t).await?;
                }
                launch(&state,&id).await?;
            },
            _=>anyhow::bail!("不支持的任务操作"),
        }
        Ok(json!({"ok":true}))
    }.await)
}
pub async fn logs(State(state):State<Shared>,Path(id):Path<String>,Query(q):Query<LogQuery>)->Response {
    shares::reply(async {shares::valid_token(&id)?;Ok(json!({"events":state.transfers.store.logs(id,q.after.max(0)).await?}))}.await)
}
pub async fn export(State(state):State<Shared>,Path(id):Path<String>)->Response {
    if shares::valid_token(&id).is_err(){return StatusCode::BAD_REQUEST.into_response()}
    let store=state.transfers.store.clone();
    let stream=futures_util::stream::try_unfold((store,id,0i64),|(store,id,after)|async move {
        let events=store.logs(id.clone(),after).await.map_err(std::io::Error::other)?;
        if events.is_empty(){return Ok::<_,std::io::Error>(None)}
        let next=events.last().unwrap()["id"].as_i64().unwrap();
        let mut text=String::new();for e in events {text.push_str(&e.to_string());text.push('\n');}
        Ok(Some((text,(store,id,next))))
    });
    ([(header::CONTENT_TYPE,"application/x-ndjson; charset=utf-8"),(header::CONTENT_DISPOSITION,"attachment; filename=transfer-log.jsonl")],Body::from_stream(stream)).into_response()
}

#[cfg(test)]
mod tests;

#[cfg(test)] mod unit_tests {
    use super::*;
    #[test] fn address_and_log_redaction(){
        let fp="A".repeat(64);let token="b".repeat(32);
        assert_eq!(address(&format!("📂 test\nantify-share://{fp}/{token}")).unwrap(),(fp,token.clone()));
        assert!(address("antify-share://bad/../../").is_err());
        assert!(!redact(&format!("https://host/?token={token}"),&token).contains(&token));
    }
}
