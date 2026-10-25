use super::*;
use crate::state::{ChatMsg, Device};
use sha2::{Digest,Sha256};

struct Fixture {root:PathBuf,state:Shared}
impl Fixture {
    fn new() -> Self {
        crate::install_crypto_provider();
        let root=std::env::temp_dir().join(format!("antify-task-test-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("downloads")).unwrap();
        let cert=rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        std::fs::write(root.join("cert.pem"),cert.cert.pem()).unwrap();
        std::fs::write(root.join("key.pem"),cert.key_pair.serialize_pem()).unwrap();
        let state=crate::server::new_state(crate::config::Identity {
            alias:"test".into(),port:std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port(),
            fingerprint:hex::encode_upper(Sha256::digest(cert.cert.der())),device_model:"test".into(),
            download_dir:root.join("downloads"),cert_pem:root.join("cert.pem"),key_pem:root.join("key.pem"),cfg_dir:root.clone(),
        }).unwrap();Self{root,state}
    }
    async fn offer(&self,to:&Fixture,path:&std::path::Path)->u64 {
        let token=uuid::Uuid::new_v4().simple().to_string();
        std::fs::create_dir_all(self.root.join("shares")).unwrap();
        // 与真实 publish 一致：存储前先 canonicalize，扫描端每轮校验根未变
        let root=path.canonicalize().unwrap();
        std::fs::write(self.root.join("shares").join(format!("{token}.json")),json!({"token":token,"root":root,"name":"project"}).to_string()).unwrap();
        to.state.upsert_device(Device{fingerprint:self.state.identity.fingerprint.clone(),alias:"source".into(),ip:"127.0.0.1".parse().unwrap(),port:self.state.identity.port,https:true,device_model:None,device_type:None,version:"2.1".into(),download:false,last_seen:crate::state::now_ms()}).await;
        to.state.push_chat(ChatMsg{peer:self.state.identity.fingerprint.clone(),kind:"text".into(),text:format!("📂 project\nantify-share://{}/{token}",self.state.identity.fingerprint),at:crate::state::now_ms(),..Default::default()}).await
    }
}
impl Drop for Fixture {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.root);}}
async fn value(response:Response)->serde_json::Value {
    let status=response.status();let bytes=axum::body::to_bytes(response.into_body(),2*1024*1024).await.unwrap();
    let v:serde_json::Value=serde_json::from_slice(&bytes).unwrap();assert!(status.is_success(),"{v}");v
}
async fn stopped(state:&Shared,id:&str)->Task {
    tokio::time::timeout(Duration::from_secs(120),async {
        loop {let t=state.transfers.store.task(id).await.unwrap();
            if !state.transfers.active.lock().await.contains_key(id){return t;}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap()
}
#[tokio::test]
async fn folder_task_pause_resume_logs_and_idempotent_accept(){
    let source=Fixture::new();let receiver=Fixture::new();
    let dir=source.root.join("source");std::fs::create_dir_all(dir.join("nested/empty")).unwrap();
    for n in 0..205 {std::fs::write(dir.join(format!("small-{n}")),b"contents").unwrap();}
    let large=vec![42u8;4*1024*1024+37];
    std::fs::write(dir.join("nested/large.bin"),&large).unwrap();
    std::fs::write(dir.join(".hidden"),b"hidden").unwrap();std::fs::write(dir.join("zero"),b"").unwrap();
    let st=source.state.clone();let server=tokio::spawn(async move{crate::server::serve(st).await.unwrap()});
    tokio::time::sleep(Duration::from_millis(100)).await;
    let message=source.offer(&receiver,&dir).await;
    let v=value(create(State(receiver.state.clone()),Json(Create{message_id:message,directory:None})).await).await;
    let id=v["task"]["id"].as_str().unwrap();
    value(control(State(receiver.state.clone()),Path((id.into(),"pause".into()))).await).await;
    let t=stopped(&receiver.state,id).await;assert_eq!(t.state,"paused");
    value(control(State(receiver.state.clone()),Path((id.into(),"resume".into()))).await).await;
    let t=stopped(&receiver.state,id).await;assert_eq!(t.state,"done","{}",t.error);
    assert_eq!(t.files,208);assert_eq!(t.completed,t.files);assert_eq!(t.received,t.bytes);
    assert_eq!(std::fs::read(t.destination.join("nested/large.bin")).unwrap(),large);
    assert!(t.destination.join("nested/empty").is_dir());assert!(t.destination.join("zero").is_file());
    assert_eq!(std::fs::read(t.destination.join(".hidden")).unwrap(),b"hidden");
    assert_eq!(receiver.state.db.recent_msgs(500).len(),1,"One task, not hundreds of chat messages");
    let again=value(create(State(receiver.state.clone()),Json(Create{message_id:message,directory:None})).await).await;
    assert_eq!(again["task"]["id"],id);
    let events=receiver.state.transfers.store.logs(id.into(),0).await.unwrap();assert_eq!(events.len(),200);
    let exported=export(State(receiver.state.clone()),Path(id.into())).await;
    let bytes=axum::body::to_bytes(exported.into_body(),2*1024*1024).await.unwrap();
    let text=String::from_utf8(bytes.to_vec()).unwrap();assert!(text.lines().count()>208);assert!(!text.contains(&t.token));
    assert!(receiver.root.join("transfers/logs").join(id).join("current.jsonl").is_file());
    server.abort();let _=server.await;
}

#[tokio::test]
async fn rejects_forwarded_invitation_and_keeps_user_pause(){
    let f=Fixture::new();
    let m=f.state.push_chat(ChatMsg{peer:"A".repeat(64),kind:"text".into(),text:format!("antify-share://{}/{}","B".repeat(64),"c".repeat(32)),..Default::default()}).await;
    let response=create(State(f.state.clone()),Json(Create{message_id:m,directory:None})).await;
    assert_eq!(response.status(),StatusCode::BAD_REQUEST);
}
