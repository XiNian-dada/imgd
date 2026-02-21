use std::{sync::Arc, time::Duration};

use axum::{
    body::Body,
    extract::connect_info::ConnectInfo,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use imgd::{build_app, config::AppConfig, AppState, Metrics, SimpleRateLimiter};
use serde_json::Value;
use tokio::sync::Semaphore;
use tower::ServiceExt;

fn make_test_state(data_dir: &std::path::Path) -> AppState {
    AppState {
        config: AppConfig {
            bind_addr: "127.0.0.1:0".parse().expect("addr"),
            upload_token: "secret".to_string(),
            public_base_url: "https://img.example.com/images".to_string(),
            data_dir: data_dir.to_path_buf(),
            max_upload_bytes: 5 * 1024 * 1024,
            max_concurrent_uploads: 4,
            rate_limit_per_minute: 100,
        },
        upload_semaphore: Arc::new(Semaphore::new(4)),
        rate_limiter: SimpleRateLimiter::new(100, Duration::from_secs(60)),
        metrics: Arc::new(Metrics::default()),
    }
}

fn webp_fixture() -> Vec<u8> {
    // Minimal header that satisfies RIFF....WEBP signature check.
    let mut data = Vec::from(*b"RIFF");
    data.extend_from_slice(&[0x10, 0x00, 0x00, 0x00]);
    data.extend_from_slice(b"WEBP");
    data.extend_from_slice(b"VP8 ");
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    data
}

fn multipart_body(boundary: &str, filename: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: image/webp\r\n\r\n");
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

async fn send_upload(app: axum::Router, filename: &str, bytes: &[u8]) -> (StatusCode, Value) {
    let boundary = "----imgd-boundary";
    let body = multipart_body(boundary, filename, bytes);

    let mut req = Request::builder()
        .method("POST")
        .uri("/upload")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header("x-upload-token", "secret")
        .body(Body::from(body))
        .expect("request");

    req.extensions_mut()
        .insert(ConnectInfo("127.0.0.1:8080".parse().expect("socket")));

    let resp = app.oneshot(req).await.expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&bytes).expect("json body");
    (status, json)
}

#[tokio::test]
async fn upload_webp_success_and_file_exists() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let state = make_test_state(tmp.path());
    let app = build_app(state);

    let (status, body) = send_upload(app, "ok.webp", &webp_fixture()).await;
    assert_eq!(status, StatusCode::OK);

    let path = body.get("path").and_then(Value::as_str).expect("path");
    let rel = path.trim_start_matches('/');
    assert!(tmp.path().join(rel).exists());
}

#[tokio::test]
async fn reject_fake_webp_text_payload() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let state = make_test_state(tmp.path());
    let app = build_app(state);

    let (status, body) = send_upload(app, "fake.webp", b"hello, world").await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        body.get("error").and_then(Value::as_str),
        Some("unsupported_media_type")
    );
}

#[tokio::test]
async fn deduplicate_same_content_by_sha256() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let state = make_test_state(tmp.path());
    let app = build_app(state);
    let bytes = webp_fixture();

    let (s1, b1) = send_upload(app.clone(), "a.webp", &bytes).await;
    let (s2, b2) = send_upload(app, "b.webp", &bytes).await;

    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(b1.get("sha256"), b2.get("sha256"));
    assert_eq!(b1.get("path"), b2.get("path"));

    let rel = b1
        .get("path")
        .and_then(Value::as_str)
        .expect("path")
        .trim_start_matches('/');
    assert!(tmp.path().join(rel).exists());
}
