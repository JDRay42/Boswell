//! Uniform JSON error type for the gateway.
//!
//! Every error response has the shape `{ "error": "...", "request_id": "..." }`
//! and carries the same id in the `x-request-id` header.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use boswell_sdk::SdkError;
use serde_json::json;

/// A gateway error carrying an HTTP status and a human-readable message.
#[derive(Debug)]
pub struct ApiError {
    /// HTTP status to return.
    pub status: StatusCode,
    /// Human-readable message (safe to expose to the client).
    pub message: String,
    /// Correlation id; filled in on response if not already set.
    pub request_id: Option<String>,
}

impl ApiError {
    /// Build an error with a specific status and message.
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            request_id: None,
        }
    }

    /// 400 Bad Request.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    /// 401 Unauthorized.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    /// 403 Forbidden.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    /// 404 Not Found.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    /// 429 Too Many Requests.
    pub fn rate_limited() -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded")
    }

    /// Attach a request id (from the request-id middleware) for correlation.
    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = self
            .request_id
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

        let body = Json(json!({
            "error": self.message,
            "request_id": request_id,
        }));

        let mut response = (self.status, body).into_response();
        if let Ok(value) = request_id.parse() {
            response.headers_mut().insert("x-request-id", value);
        }
        response
    }
}

/// Map SDK (upstream) errors to gateway HTTP statuses. These are failures
/// talking to the Router / gRPC instance, not client faults, so they map to
/// 5xx.
impl From<SdkError> for ApiError {
    fn from(e: SdkError) -> Self {
        let status = match &e {
            SdkError::NotConnected
            | SdkError::ConnectionError(_)
            | SdkError::NoInstancesAvailable => StatusCode::SERVICE_UNAVAILABLE,
            SdkError::AuthError(_)
            | SdkError::RouterError(_)
            | SdkError::GrpcError(_)
            | SdkError::SessionError(_) => StatusCode::BAD_GATEWAY,
        };
        ApiError::new(status, format!("Upstream error: {}", e))
    }
}
