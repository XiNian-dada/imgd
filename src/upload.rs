use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::Instant,
};

use axum::{
    extract::{connect_info::ConnectInfo, Multipart, State},
    http::HeaderMap,
    Json,
};
use chrono::{Datelike, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{error::AppError, webp, AppState};

#[derive(Serialize)]
pub struct UploadResponse {
    pub url: String,
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

pub async fn upload_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, AppError> {
    let started = Instant::now();
    let ip = addr.ip();
    let request_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");

    while let Some(field) = multipart.next_field().await? {
        if field.name() != Some("file") {
            state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
            warn!(ip = %ip, request_id, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "invalid_field", "upload rejected");
            return Err(AppError::BadRequest);
        }

        let filename = field.file_name().ok_or_else(|| {
            state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
            warn!(ip = %ip, request_id, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "missing_filename", "upload rejected");
            AppError::BadRequest
        })?;

        if !webp::has_webp_extension(filename) {
            state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
            warn!(ip = %ip, request_id, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "extension", "upload rejected");
            return Err(AppError::UnsupportedMediaType);
        }

        let tmp_dir = state.config.data_dir.join(".tmp");
        fs::create_dir_all(&tmp_dir).await?;

        let tmp_name = format!(".uploading-{}", Uuid::new_v4());
        let tmp_path = tmp_dir.join(tmp_name);

        let mut writer = create_new_file(&tmp_path).await?;
        let mut hasher = Sha256::new();
        let mut header = Vec::with_capacity(12);
        let mut size: u64 = 0;

        let mut field = field;
        loop {
            let chunk = match field.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(_) => {
                    let _ = fs::remove_file(&tmp_path).await;
                    state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
                    warn!(ip = %ip, request_id, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "multipart_read", "upload rejected");
                    return Err(AppError::BadRequest);
                }
            };

            size = size.saturating_add(chunk.len() as u64);
            if size > state.config.max_upload_bytes as u64 {
                let _ = fs::remove_file(&tmp_path).await;
                state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
                warn!(ip = %ip, request_id, size, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "too_large", "upload rejected");
                return Err(AppError::FileTooLarge);
            }

            if header.len() < 12 {
                let need = 12 - header.len();
                let take = need.min(chunk.len());
                header.extend_from_slice(&chunk[..take]);
            }

            if writer.write_all(&chunk).await.is_err() {
                let _ = fs::remove_file(&tmp_path).await;
                state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
                error!(ip = %ip, request_id, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "tmp_write", "upload failed");
                return Err(AppError::Internal);
            }
            hasher.update(&chunk);
        }

        if writer.flush().await.is_err() {
            let _ = fs::remove_file(&tmp_path).await;
            state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
            error!(ip = %ip, request_id, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "tmp_flush", "upload failed");
            return Err(AppError::Internal);
        }
        drop(writer);

        if !webp::has_webp_signature(&header) {
            let _ = fs::remove_file(&tmp_path).await;
            state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
            warn!(ip = %ip, request_id, size, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "signature", "upload rejected");
            return Err(AppError::UnsupportedMediaType);
        }

        let sha256 = hex::encode(hasher.finalize());
        let now = Utc::now();
        let year = now.year();
        let month = now.month();

        let relative = format!("/{year:04}/{month:02}/{sha256}.webp");
        let final_dir = state
            .config
            .data_dir
            .join(format!("{year:04}"))
            .join(format!("{month:02}"));
        if fs::create_dir_all(&final_dir).await.is_err() {
            let _ = fs::remove_file(&tmp_path).await;
            state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
            error!(ip = %ip, request_id, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "mkdir_final", "upload failed");
            return Err(AppError::Internal);
        }
        let final_path = final_dir.join(format!("{sha256}.webp"));

        match fs::try_exists(&final_path).await {
            Ok(true) => {
                let _ = fs::remove_file(&tmp_path).await;
            }
            Ok(false) => match fs::rename(&tmp_path, &final_path).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&tmp_path).await;
                }
                Err(err) => {
                    error!(ip = %ip, request_id, error = %err, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "rename_final", "upload failed");
                    let _ = fs::remove_file(&tmp_path).await;
                    state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
                    return Err(AppError::Internal);
                }
            },
            Err(err) => {
                error!(ip = %ip, request_id, error = %err, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "check_final_exists", "upload failed");
                let _ = fs::remove_file(&tmp_path).await;
                state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
                return Err(AppError::Internal);
            }
        }

        state.metrics.upload_ok.fetch_add(1, Ordering::Relaxed);

        let url = format!(
            "{}{}",
            state.config.public_base_url.trim_end_matches('/'),
            relative
        );
        info!(
            ip = %ip,
            request_id,
            sha256 = %sha256,
            size,
            path = %relative,
            elapsed_ms = started.elapsed().as_millis(),
            result = "ok",
            "upload finished"
        );

        return Ok(Json(UploadResponse {
            url,
            path: relative,
            sha256,
            size,
        }));
    }

    state.metrics.upload_fail.fetch_add(1, Ordering::Relaxed);
    warn!(ip = %ip, request_id, elapsed_ms = started.elapsed().as_millis(), result = "fail", reason = "missing_file", "upload rejected");
    Err(AppError::BadRequest)
}

async fn create_new_file(path: &Path) -> Result<File, AppError> {
    Ok(OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(PathBuf::from(path))
        .await?)
}
