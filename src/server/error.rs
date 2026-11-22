//! Typed handler failures. Every variant renders a full HTML error page; the
//! message never contains a cookie, password or CSRF token.

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use crate::error::Error;
use crate::render;

/// Handler result: an `AppError` renders a full HTML error page.
pub(super) type Handled<T> = std::result::Result<T, AppError>;

pub(super) enum AppError {
    BadRequest(String),
    Forbidden(String),
    NotFound,
    Upstream(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "Not cached. Open the work from the start page first.".to_string(),
            ),
            Self::Upstream(message) => (StatusCode::BAD_GATEWAY, message),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        (status, Html(render::error_page(status.as_u16(), &message))).into_response()
    }
}

impl From<Error> for AppError {
    fn from(error: Error) -> Self {
        match error {
            Error::Config(message)
            | Error::Io(message)
            | Error::Store(message)
            | Error::Crypto(message) => Self::Internal(message),
            Error::Network(message) | Error::UpstreamChanged(message) => Self::Upstream(message),
            Error::RateLimited { retry_after } => Self::Upstream(match retry_after {
                Some(secs) => format!("AO3 is rate limiting this instance; retry after {secs}s"),
                None => "AO3 is rate limiting this instance; wait and retry".to_string(),
            }),
            Error::BlockedByEdge { status } => {
                Self::Upstream(format!("AO3 blocked the request (HTTP {status})"))
            }
            Error::Validation(message) => Self::BadRequest(message),
            Error::SpamRejected => Self::BadRequest(
                "AO3's spam filter rejected the content; reword and try again.".into(),
            ),
            Error::LoggedOut | Error::SessionIncomplete => Self::Upstream(
                "The AO3 session is not usable. Import cookies on the Session page.".to_string(),
            ),
            Error::CsrfMissing | Error::CsrfExpired => Self::Upstream(
                "AO3 rejected the CSRF token. Verify the session on the Session page.".to_string(),
            ),
            Error::LoginRejected(message) => {
                Self::Upstream(format!("AO3 rejected the login: {message}"))
            }
            Error::PasswordChangeRequired => Self::Upstream(
                "AO3 requires a password change before this session can be used.".to_string(),
            ),
            Error::LogoutFailed(message) => {
                Self::Upstream(format!("Logout did not complete upstream: {message}"))
            }
            Error::Killed => Self::Internal("Upstream traffic is disabled by the operator".into()),
        }
    }
}
