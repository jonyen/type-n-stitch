//! One error type for every handler, rendered as `{ "error": "..." }`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub struct AppError {
    status: StatusCode,
    message: String,
    /// Which item in a submitted batch was rejected, if any.
    index: Option<usize>,
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            index: None,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
            index: None,
        }
    }

    /// A downstream tool (ffmpeg, whisper, TTS) failed or is unreachable.
    pub fn upstream(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
            index: None,
        }
    }

    /// The server got itself into an inconsistent state; the client cannot
    /// fix it by sending something else.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
            index: None,
        }
    }

    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "sign in first".into(),
            index: None,
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
            index: None,
        }
    }

    /// The request collided with concurrent state (e.g. a unique-key race)
    /// and the client can plausibly succeed by retrying.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
            index: None,
        }
    }

    /// A bad request that names which entry of a batch was rejected.
    pub fn bad_request_at(index: usize, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            index: Some(index),
        }
    }

    /// A multipart body that could not be read, keeping axum's status: 413
    /// when it ran past the upload limit, 400 when it was malformed.
    /// `context` goes before axum's own words.
    pub fn multipart(err: &axum::extract::multipart::MultipartError, context: &str) -> Self {
        let text = err.body_text();
        Self {
            status: err.status(),
            message: if context.is_empty() {
                text
            } else {
                format!("{context}: {text}")
            },
            index: None,
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }
}

/// The message a client would see, so callers that are not HTTP handlers —
/// the MCP tools — can pass the server's own wording straight through.
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.index {
            Some(index) => write!(f, "{} (at {index})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(err: E) -> Self {
        let err: anyhow::Error = err.into();
        tracing::error!("{err:#}");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("{err:#}"),
            index: None,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message });
        if let Some(index) = self.index {
            body["index"] = json!(index);
        }
        (self.status, Json(body)).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
