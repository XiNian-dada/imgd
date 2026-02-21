use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("unsupported_media_type")]
    UnsupportedMediaType,
    #[error("file_too_large")]
    FileTooLarge,
    #[error("bad_request")]
    BadRequest,
    #[error("too_many_requests")]
    TooManyRequests,
    #[error("internal_error")]
    Internal,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, error, detail) = match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", None),
            AppError::UnsupportedMediaType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                None,
            ),
            AppError::FileTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "file_too_large", None),
            AppError::BadRequest => (StatusCode::BAD_REQUEST, "bad_request", None),
            AppError::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "too_many_requests", None),
            AppError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", None),
        };

        (status, Json(ErrorBody { error, detail })).into_response()
    }
}

impl From<std::io::Error> for AppError {
    fn from(_: std::io::Error) -> Self {
        AppError::Internal
    }
}

impl From<axum::extract::multipart::MultipartError> for AppError {
    fn from(_: axum::extract::multipart::MultipartError) -> Self {
        AppError::BadRequest
    }
}
