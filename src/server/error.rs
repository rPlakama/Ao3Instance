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
    Unavailable(String),
    /// Upstream rate limit through the local solver.
    TooManyRequests(String),
    /// AO3's edge refused a work; the page renders the explicit solve card.
    Blocked {
        status: u16,
        work_id: String,
        solver_reachable: bool,
        send_cookies_allowed: bool,
    },
    /// AO3's edge refused a search; offer an explicit solve-and-retry.
    BlockedSearch {
        status: u16,
        target: String,
        retry: String,
        solver_available: bool,
        send_cookies_allowed: bool,
    },
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::Blocked {
                status,
                work_id,
                solver_reachable,
                send_cookies_allowed,
            } => (
                StatusCode::BAD_GATEWAY,
                Html(render::challenge_error_page(
                    status,
                    &work_id,
                    solver_reachable,
                    send_cookies_allowed,
                )),
            )
                .into_response(),
            Self::BlockedSearch {
                status,
                target,
                retry,
                solver_available,
                send_cookies_allowed,
            } => (
                StatusCode::BAD_GATEWAY,
                Html(render::search_challenge_page(
                    status,
                    &target,
                    &retry,
                    solver_available,
                    send_cookies_allowed,
                )),
            )
                .into_response(),
            Self::BadRequest(message) => html_error(StatusCode::BAD_REQUEST, &message),
            Self::Forbidden(message) => html_error(StatusCode::FORBIDDEN, &message),
            Self::NotFound => html_error(
                StatusCode::NOT_FOUND,
                "Not cached. Open the work from the start page first.",
            ),
            Self::Upstream(message) => html_error(StatusCode::BAD_GATEWAY, &message),
            Self::Unavailable(message) => html_error(StatusCode::SERVICE_UNAVAILABLE, &message),
            Self::TooManyRequests(message) => html_error(StatusCode::TOO_MANY_REQUESTS, &message),
            Self::Internal(message) => html_error(StatusCode::INTERNAL_SERVER_ERROR, &message),
        }
    }
}

fn html_error(status: StatusCode, message: &str) -> Response {
    (status, Html(render::error_page(status.as_u16(), message))).into_response()
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
            Error::SolverDisabled => Self::Unavailable(
                "The local FlareSolverr fallback is disabled (AO3I_FLARESOLVERR=off).".into(),
            ),
            Error::SolverUnavailable(message) => Self::Unavailable(format!(
                "The local FlareSolverr is unavailable: {message}. Is the host service running?"
            )),
            Error::SolverCaptcha => Self::Upstream(
                "The local FlareSolverr hit a captcha it cannot solve; open the page in your \
                 browser instead."
                    .into(),
            ),
            Error::SolverFailed(message) => {
                Self::Upstream(format!("The local FlareSolverr failed: {message}"))
            }
            Error::SolverBodyCap => Self::Upstream(
                "The local FlareSolverr returned an oversized response; it was discarded.".into(),
            ),
        }
    }
}
