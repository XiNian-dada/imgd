pub mod auth;
pub mod config;
pub mod error;
pub mod token;
pub mod upload;
pub mod webp;

use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::{connect_info::ConnectInfo, DefaultBodyLimit, Request, State},
    http::HeaderName,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use tokio::sync::Semaphore;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

use crate::{
    auth::auth_middleware, config::AppConfig, error::AppError, token::AuthorizedToken,
    upload::upload_handler,
};

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub upload_semaphore: Arc<Semaphore>,
    pub rate_limiter: SimpleRateLimiter,
    pub token_store: crate::token::TokenStore,
    pub metrics: Arc<Metrics>,
}

#[derive(Default)]
pub struct Metrics {
    pub upload_ok: std::sync::atomic::AtomicU64,
    pub upload_fail: std::sync::atomic::AtomicU64,
    pub upload_limited: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
pub struct SimpleRateLimiter {
    window: Duration,
    inner: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl SimpleRateLimiter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn check(&self, key: String, max_requests: usize) -> bool {
        let mut guard = self.inner.lock().expect("rate limiter poisoned");
        let now = Instant::now();
        let queue = guard.entry(key).or_default();

        while let Some(front) = queue.front() {
            if now.duration_since(*front) > self.window {
                queue.pop_front();
            } else {
                break;
            }
        }

        if queue.len() >= max_requests {
            return false;
        }

        queue.push_back(now);
        true
    }
}

#[derive(Serialize)]
struct MetricsResponse {
    upload_ok: u64,
    upload_fail: u64,
    upload_limited: u64,
}

pub fn build_app(state: AppState) -> Router {
    let request_id_header = HeaderName::from_static("x-request-id");
    let protected = Router::new()
        .route("/upload", post(upload_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            concurrency_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(DefaultBodyLimit::max(
            state.config.max_upload_bytes + 1024 * 1024,
        ));

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/metrics", get(metrics_handler))
        .merge(protected)
        .with_state(state)
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
        .layer(TraceLayer::new_for_http())
}

pub fn with_connect_info(
    router: Router,
) -> axum::extract::connect_info::IntoMakeServiceWithConnectInfo<Router, SocketAddr> {
    router.into_make_service_with_connect_info::<SocketAddr>()
}

async fn metrics_handler(State(state): State<AppState>) -> Json<MetricsResponse> {
    use std::sync::atomic::Ordering;

    Json(MetricsResponse {
        upload_ok: state.metrics.upload_ok.load(Ordering::Relaxed),
        upload_fail: state.metrics.upload_fail.load(Ordering::Relaxed),
        upload_limited: state.metrics.upload_limited.load(Ordering::Relaxed),
    })
}

async fn concurrency_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: middleware::Next,
) -> Response {
    match state.upload_semaphore.clone().try_acquire_owned() {
        Ok(_permit) => next.run(req).await,
        Err(_) => {
            state
                .metrics
                .upload_limited
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            AppError::TooManyRequests.into_response()
        }
    }
}

async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: middleware::Next,
) -> Response {
    let ip = extract_ip(&req);
    if !state
        .rate_limiter
        .check(format!("ip:{ip}"), state.config.rate_limit_per_minute)
    {
        state
            .metrics
            .upload_limited
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return AppError::TooManyRequests.into_response();
    }

    if let Some(auth) = req.extensions().get::<AuthorizedToken>() {
        if let Some(per_token_limit) = auth.rate_limit_per_minute {
            if !state
                .rate_limiter
                .check(format!("token:{}", auth.token_id), per_token_limit)
            {
                state
                    .metrics
                    .upload_limited
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return AppError::TooManyRequests.into_response();
            }
        }
    }
    next.run(req).await
}

pub fn extract_ip(req: &Request<Body>) -> std::net::IpAddr {
    if let Some(raw) = req.headers().get("x-forwarded-for") {
        if let Ok(v) = raw.to_str() {
            if let Some(first) = v.split(',').next() {
                if let Ok(ip) = first.trim().parse::<std::net::IpAddr>() {
                    return ip;
                }
            }
        }
    }

    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip();
    }

    std::net::IpAddr::from([127, 0, 0, 1])
}
