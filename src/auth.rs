use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap},
    middleware,
    response::{IntoResponse, Response},
};

use crate::{error::AppError, token::AuthorizedToken, AppState};

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: middleware::Next,
) -> Response {
    if let Some(raw_token) = extract_token(req.headers()) {
        if let Some(authorized) = state.token_store.authorize(&raw_token) {
            req.extensions_mut().insert::<AuthorizedToken>(authorized);
            return next.run(req).await;
        }
    }

    AppError::Unauthorized.into_response()
}

pub fn extract_token(headers: &HeaderMap) -> Option<String> {
    if let Some(token) = headers
        .get("x-upload-token")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
    {
        return Some(token.to_owned());
    }

    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
    {
        if let Some(token) = value.strip_prefix("Bearer ") {
            if !token.is_empty() {
                return Some(token.to_owned());
            }
        }
    }

    None
}

pub fn is_authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    if let Some(raw) = extract_token(headers) {
        return raw == expected_token;
    }
    false
}

#[cfg(test)]
mod tests {
    use axum::http::{header, HeaderMap, HeaderValue};

    use super::is_authorized;

    #[test]
    fn unauthorized_without_token() {
        let headers = HeaderMap::new();
        assert!(!is_authorized(&headers, "secret"));
    }

    #[test]
    fn unauthorized_with_wrong_token() {
        let mut headers = HeaderMap::new();
        headers.insert("x-upload-token", HeaderValue::from_static("wrong"));
        assert!(!is_authorized(&headers, "secret"));
    }

    #[test]
    fn authorized_with_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );
        assert!(is_authorized(&headers, "secret"));
    }
}
