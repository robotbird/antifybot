use crate::{now, protocol::FileInfo, ApiError, ApiResult, App, Auth};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

#[derive(Deserialize)]
pub(crate) struct Upload {
    to: String,
    name: String,
}

pub(crate) async fn upload(
    State(app): State<App>,
    Auth(owner): Auth,
    Query(q): Query<Upload>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Json<FileInfo>> {
    if q.to == owner || !app.has_device(&q.to) {
        return Err(ApiError::bad("invalid recipient"));
    }
    if q.name.is_empty()
        || q.name.len() > 255
        || q.name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err(ApiError::bad("invalid file name"));
    }
    let size = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| {
            ApiError(
                StatusCode::LENGTH_REQUIRED,
                "Content-Length required".into(),
            )
        })?;
    if size > app.0.config.max_file_bytes {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file too large".into(),
        ));
    }
    let expected = headers
        .get("x-file-sha256")
        .map(|v| v.to_str().unwrap_or("").to_ascii_lowercase());
    if expected
        .as_ref()
        .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(ApiError::bad("invalid SHA-256"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let expires_at = now() + app.0.config.file_ttl_secs as i64;
    let quota = app.0.config.max_storage_bytes;
    let (fid, name, recipient) = (id.clone(), q.name.clone(), q.to.clone());
    app.0.db.run(move |db| {
        let tx = db.transaction()?;
        let used: i64 = tx.query_row("SELECT COALESCE(SUM(size),0) FROM files", [], |r| r.get(0))?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        if (used as u64).saturating_add(size) > quota || count >= 10000 { return Err(ApiError(StatusCode::INSUFFICIENT_STORAGE, "file storage quota reached".into())); }
        tx.execute("INSERT INTO files(id,owner,recipient,name,size,expires_at) VALUES(?1,?2,?3,?4,?5,?6)", params![fid,owner,recipient,name,size as i64,expires_at])?;
        tx.commit()?;
        Ok(())
    }).await?;
    let write = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(app.file_path(&id, ".part"))
            .await?;
        let mut stream = body.into_data_stream();
        let mut received = 0u64;
        let mut hash = Sha256::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| ApiError::bad("upload interrupted"))?;
            received = received.saturating_add(chunk.len() as u64);
            if received > size {
                return Err(ApiError::bad("size mismatch"));
            }
            hash.update(&chunk);
            file.write_all(&chunk).await?;
        }
        if received != size {
            return Err(ApiError::bad("size mismatch"));
        }
        let digest = format!("{:x}", hash.finalize());
        if expected.as_ref().is_some_and(|e| *e != digest) {
            return Err(ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                "SHA-256 mismatch".into(),
            ));
        }
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(app.file_path(&id, ".part"), app.file_path(&id, "")).await?;
        let (fid, saved_hash) = (id.clone(), digest.clone());
        app.0
            .db
            .run(move |db| {
                db.execute(
                    "UPDATE files SET ready=1,sha256=?2 WHERE id=?1",
                    params![fid, saved_hash],
                )?;
                Ok(())
            })
            .await?;
        Ok::<_, ApiError>(digest)
    };
    let result = tokio::time::timeout(Duration::from_secs(300), write)
        .await
        .unwrap_or_else(|_| {
            Err(ApiError(
                StatusCode::REQUEST_TIMEOUT,
                "upload timed out".into(),
            ))
        });
    match result {
        Ok(sha256) => Ok(Json(FileInfo {
            id,
            name: q.name,
            size,
            sha256,
            expires_at,
        })),
        Err(e) => {
            for suffix in [".part", ""] {
                let _ = tokio::fs::remove_file(app.file_path(&id, suffix)).await;
            }
            let _ = app.0.db.remove_file(id).await;
            Err(e)
        }
    }
}

async fn metadata(app: &App, device: String, id: String) -> ApiResult<FileInfo> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| ApiError::bad("invalid file ID"))?
        .to_string();
    let fid = id.clone();
    app.0.db.run(move |db| {
        db.query_row("SELECT name,size,sha256,expires_at FROM files WHERE id=?1 AND (owner=?2 OR recipient=?2) AND ready=1 AND expires_at>?3",
            params![fid,device,now()], |r| Ok(FileInfo { id:fid.clone(), name:r.get(0)?, size:r.get::<_,i64>(1)? as u64, sha256:r.get(2)?, expires_at:r.get(3)? }))
            .optional()?.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "file unavailable".into()))
    }).await
}

pub(crate) async fn info(
    State(app): State<App>,
    Auth(device): Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<FileInfo>> {
    Ok(Json(metadata(&app, device, id).await?))
}

pub(crate) async fn download(
    State(app): State<App>,
    Auth(device): Auth,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let info = metadata(&app, device, id).await?;
    let file = tokio::fs::File::open(app.file_path(&info.id, "")).await?;
    let mut response = Body::from_stream(tokio_util::io::ReaderStream::new(file)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    headers.insert(header::CONTENT_DISPOSITION, "attachment".parse().unwrap());
    headers.insert(header::CONTENT_LENGTH, info.size.into());
    headers.insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert(
        "x-file-sha256",
        info.sha256.parse().map_err(ApiError::internal)?,
    );
    Ok(response)
}
