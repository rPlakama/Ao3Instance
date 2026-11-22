#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Session state-machine tests: every path in `docs/write-client-plan.md` §2.
//!
//! These are integration tests on purpose — the thing being proven is the
//! interaction between transport, cookie jar and state classification.

mod support;

use ao3instance::ao3::jar::Jar;
use ao3instance::ao3::session::{AccountSession, SessionState};
use ao3instance::config::Config;
use ao3instance::crypto::SecretString;
use ao3instance::error::Error;
use support::{Response, Server};

const LOGIN_HTML: &str = r#"<!DOCTYPE html><html><head>
<meta name="csrf-token" content="login-token" />
</head><body><div id="loginform"></div></body></html>"#;

const LOGIN_HTML_NO_TOKEN: &str =
    r#"<!DOCTYPE html><html><head></head><body><div id="loginform"></div></body></html>"#;

/// Clone-shaped flash slot: the session layout renders `class="flash alert"`
/// and Devise uses the alert slot for failed logins.
const REJECT_HTML: &str = r#"<div class="flash alert" role="alert">
<p>Invalid login or password.</p>
</div>"#;

const TOKEN_JSON: &str = r#"{"token":"action-token"}"#;

fn config_for(server: &Server) -> Config {
    let base = server.url();
    Config::from_pairs([
        ("AO3I_BASE_URL", base.as_str()),
        ("AO3I_DATA_DIR", "/tmp/ao3i-test"),
        ("XDG_CONFIG_HOME", "/tmp/xdg-test"),
        ("AO3I_MIN_SPACING_MS", "0"),
    ])
    .expect("test config")
}

fn complete_jar() -> Jar {
    let mut jar = Jar::new();
    jar.apply("_otwarchive_session=session-value; Path=/; HttpOnly");
    jar.apply("user_credentials=1; Path=/");
    jar
}

fn session_only_jar() -> Jar {
    let mut jar = Jar::new();
    jar.apply("_otwarchive_session=session-value; Path=/; HttpOnly");
    jar
}

/// Routes for a successful password login of `login`.
fn login_routes(login: &str) -> Vec<(String, Response)> {
    vec![
        ("GET /users/login".to_string(), Response::ok(LOGIN_HTML)),
        (
            "POST /users/login".to_string(),
            Response::new(302)
                .with_header("location", format!("/users/{login}"))
                .with_cookie("_otwarchive_session=rotated; Path=/; HttpOnly")
                .with_cookie("user_credentials=1; Path=/"),
        ),
        (
            format!("GET /users/{login}/subscriptions"),
            Response::ok("<html>subscriptions</html>"),
        ),
        (
            "GET /token_dispenser.json".to_string(),
            Response::json(TOKEN_JSON),
        ),
    ]
}

/// Routes that prove an already-imported jar.
fn probe_routes(login: &str) -> Vec<(String, Response)> {
    vec![
        (
            format!("GET /users/{login}/subscriptions"),
            Response::ok("<html>subscriptions</html>"),
        ),
        (
            "GET /token_dispenser.json".to_string(),
            Response::json(TOKEN_JSON),
        ),
    ]
}

async fn active_session(server: &Server, login: &str) -> AccountSession {
    let mut session = AccountSession::new(&config_for(server)).expect("session");
    let state = session.import(login, complete_jar()).await.expect("import");
    assert_eq!(state, SessionState::Active);
    session
}

#[tokio::test]
async fn login_success_reaches_active_and_gets_a_token() {
    let server = Server::start(login_routes("someone"));
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let state = session
        .login("someone", SecretString::new("hunter2"))
        .await
        .expect("login");
    assert_eq!(state, SessionState::Active);
    assert_eq!(session.account(), "someone");
    assert!(session.jar().is_complete());
    assert_eq!(session.csrf_token().expect("token"), "action-token");

    let requests = server.requests();
    assert_eq!(requests.len(), 4, "login page, post, probe, token");
    assert_eq!(requests[1].method, "POST");
    let body = &requests[1].body;
    assert!(body.contains("user%5Blogin%5D=someone"));
    assert!(body.contains("authenticity_token=login-token"));
    let probe = requests
        .iter()
        .find(|request| request.path == "/users/someone/subscriptions")
        .expect("probe request");
    assert_eq!(
        probe.header("cookie"),
        Some("_otwarchive_session=rotated; user_credentials=1")
    );
}

#[tokio::test]
async fn rejected_login_200_is_typed_and_wipes_the_jar() {
    let server = Server::start(vec![
        ("GET /users/login", Response::ok(LOGIN_HTML)),
        ("POST /users/login", Response::ok(REJECT_HTML)),
    ]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .login("someone", SecretString::new("wrong"))
        .await
        .expect_err("login must fail");
    let Error::LoginRejected(message) = err else {
        panic!("unexpected error: {err:?}");
    };
    assert_eq!(message, "Invalid login or password.");
    assert_eq!(session.state(), SessionState::LoggedOut);
    assert!(session.jar().is_empty());
    assert!(session.csrf_token().is_err());
}

#[tokio::test]
async fn rejected_login_302_to_login_page_is_not_success() {
    let server = Server::start(vec![
        ("GET /users/login", Response::ok(LOGIN_HTML)),
        (
            "POST /users/login",
            Response::redirect("/users/login?return_to=%2Fusers%2Fsomeone"),
        ),
    ]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .login("someone", SecretString::new("wrong"))
        .await
        .expect_err("302 back to login is a rejection");
    assert!(matches!(err, Error::LoginRejected(_)));
    assert_eq!(session.state(), SessionState::LoggedOut);
    assert!(session.jar().is_empty());
}

#[tokio::test]
async fn csrf_failure_on_login_is_csrf_expired_not_success() {
    let server = Server::start(vec![
        ("GET /users/login", Response::ok(LOGIN_HTML)),
        ("POST /users/login", Response::redirect("/auth_error")),
    ]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .login("someone", SecretString::new("hunter2"))
        .await
        .expect_err("auth_error redirect");
    assert!(matches!(err, Error::CsrfExpired));
    assert_eq!(session.state(), SessionState::LoggedOut);
}

#[tokio::test]
async fn pwned_password_redirect_is_its_own_outcome() {
    let server = Server::start(vec![
        ("GET /users/login", Response::ok(LOGIN_HTML)),
        (
            "POST /users/login",
            Response::redirect("/users/someone/change_password"),
        ),
    ]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .login("someone", SecretString::new("hunter2"))
        .await
        .expect_err("password change required");
    assert!(matches!(err, Error::PasswordChangeRequired));
    assert_eq!(session.state(), SessionState::LoggedOut);
    assert!(session.jar().is_empty(), "must not keep the signed-in jar");
}

#[tokio::test]
async fn email_login_uses_the_canonical_login_from_the_redirect() {
    let server = Server::start(login_routes("real_login"));
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let state = session
        .login("mail@example.com", SecretString::new("hunter2"))
        .await
        .expect("login");
    assert_eq!(state, SessionState::Active);
    assert_eq!(session.account(), "real_login");
    assert!(
        server
            .requests()
            .iter()
            .any(|request| request.path == "/users/real_login/subscriptions")
    );
}

#[tokio::test]
async fn login_name_containing_magic_words_is_still_a_user() {
    // A user called `change_password_fan` must not be sent to a password change.
    let server = Server::start(login_routes("change_password_fan"));
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let state = session
        .login("change_password_fan", SecretString::new("hunter2"))
        .await
        .expect("login");
    assert_eq!(state, SessionState::Active);
    assert_eq!(session.account(), "change_password_fan");
}

#[tokio::test]
async fn off_origin_login_redirect_is_drift_and_wipes_the_jar() {
    let server = Server::start(vec![
        ("GET /users/login", Response::ok(LOGIN_HTML)),
        (
            "POST /users/login",
            Response::redirect("https://evil.example/users/someone"),
        ),
    ]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .login("someone", SecretString::new("hunter2"))
        .await
        .expect_err("off-origin redirect must not be trusted");
    assert!(matches!(err, Error::UpstreamChanged(_)));
    assert_eq!(session.state(), SessionState::LoggedOut);
    assert!(session.jar().is_empty());
}

#[tokio::test]
async fn probe_after_logged_out_refuses_without_network() {
    let server = Server::start(vec![(
        "GET /users/someone/subscriptions",
        Response::redirect("/users/login?return_to=%2Fusers%2Fsomeone"),
    )]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");
    session
        .import("someone", complete_jar())
        .await
        .expect("import");
    let requests_after_import = server.requests().len();

    let err = session
        .probe()
        .await
        .expect_err("dead session cannot be probed");
    assert!(matches!(err, Error::LoggedOut));
    assert_eq!(server.requests().len(), requests_after_import);
}

#[tokio::test]
async fn incomplete_import_is_rejected_without_network_or_retention() {
    let server = Server::empty();
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let state = session
        .import("someone", session_only_jar())
        .await
        .expect("import");
    assert_eq!(state, SessionState::SessionIncomplete);
    assert!(server.requests().is_empty(), "not even a probe may be sent");
    assert!(session.jar().is_empty(), "a rejected jar must be wiped");
}

#[tokio::test]
async fn imported_jar_is_proven_by_a_read_then_gets_a_token() {
    let server = Server::start(probe_routes("someone"));
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let state = session
        .import("someone", complete_jar())
        .await
        .expect("import");
    assert_eq!(state, SessionState::Active);
    assert_eq!(session.csrf_token().expect("token"), "action-token");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn probe_recognises_logged_out_and_wipes_the_jar() {
    let server = Server::start(vec![(
        "GET /users/someone/subscriptions",
        Response::redirect("/users/login?return_to=%2Fusers%2Fsomeone%2Fsubscriptions"),
    )]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let state = session
        .import("someone", complete_jar())
        .await
        .expect("import");
    assert_eq!(state, SessionState::LoggedOut);
    assert!(session.jar().is_empty());
    assert!(session.csrf_token().is_err());
}

#[tokio::test]
async fn probe_recognises_incomplete_cookie_jar_and_wipes_it() {
    let server = Server::start(vec![(
        "GET /users/someone/subscriptions",
        Response::redirect("/lost_cookie"),
    )]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let state = session
        .import("someone", complete_jar())
        .await
        .expect("import");
    assert_eq!(state, SessionState::SessionIncomplete);
    assert!(session.jar().is_empty());
}

#[tokio::test]
async fn probe_unknown_redirect_fails_closed() {
    let server = Server::start(vec![(
        "GET /users/someone/subscriptions",
        Response::redirect("/somewhere/new"),
    )]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .import("someone", complete_jar())
        .await
        .expect_err("unknown redirect must not be interpreted");
    assert!(matches!(err, Error::UpstreamChanged(_)));
    assert_eq!(session.state(), SessionState::Imported);
    assert!(!session.state().is_usable());
}

#[tokio::test]
async fn cookie_rotation_drops_the_csrf_token() {
    let server = Server::start_sequence(vec![
        (
            "GET /users/someone/subscriptions".to_string(),
            vec![
                Response::ok("<html>subscriptions</html>"),
                Response::ok("<html>subscriptions</html>")
                    .with_cookie("_otwarchive_session=rotated; Path=/; HttpOnly"),
            ],
        ),
        (
            "GET /token_dispenser.json".to_string(),
            vec![Response::json(TOKEN_JSON)],
        ),
    ]);
    let mut session = active_session(&server, "someone").await;
    assert!(session.csrf_token().is_ok());

    let proof = session.probe().await.expect("second probe");
    assert_eq!(
        proof,
        SessionState::Active,
        "the read still proves liveness"
    );
    assert_eq!(
        session.state(),
        SessionState::Imported,
        "a verified jar without a token is not Active"
    );
    assert!(!session.state().is_usable());
    assert!(
        session.csrf_token().is_err(),
        "a rotated jar must invalidate the old token"
    );
}

#[tokio::test]
async fn logout_sends_the_csrf_token_and_wipes_locally() {
    let mut routes = probe_routes("someone");
    routes.push((
        "DELETE /users/logout".to_string(),
        Response::new(302).with_header("location", "/"),
    ));
    let server = Server::start(routes);
    let mut session = active_session(&server, "someone").await;

    let state = session.logout().await.expect("logout");
    assert_eq!(state, SessionState::LoggedOut);
    assert!(session.jar().is_empty());
    assert!(session.csrf_token().is_err());

    let request = server.requests().last().cloned().expect("a request");
    assert_eq!(request.method, "DELETE");
    assert_eq!(request.header("x-csrf-token"), Some("action-token"));
    assert!(request.body.contains("authenticity_token=action-token"));
}

#[tokio::test]
async fn logout_refreshes_a_stale_token_and_retries_once() {
    let server = Server::start_sequence(vec![
        (
            "GET /users/someone/subscriptions".to_string(),
            vec![Response::ok("<html>subscriptions</html>")],
        ),
        (
            "GET /token_dispenser.json".to_string(),
            vec![
                Response::json(TOKEN_JSON),
                Response::json(r#"{"token":"fresh-token"}"#),
            ],
        ),
        (
            "DELETE /users/logout".to_string(),
            vec![
                Response::redirect("/auth_error"),
                Response::new(302).with_header("location", "/"),
            ],
        ),
    ]);
    let mut session = active_session(&server, "someone").await;

    let state = session.logout().await.expect("retry with a fresh token");
    assert_eq!(state, SessionState::LoggedOut);
    assert!(session.jar().is_empty());

    let deletes: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|request| request.method == "DELETE")
        .collect();
    assert_eq!(deletes.len(), 2, "exactly one retry");
    assert_eq!(deletes[0].header("x-csrf-token"), Some("action-token"));
    assert_eq!(deletes[1].header("x-csrf-token"), Some("fresh-token"));
}

#[tokio::test]
async fn logout_gives_up_after_one_refresh() {
    let server = Server::start_sequence(vec![
        (
            "GET /users/someone/subscriptions".to_string(),
            vec![Response::ok("<html>subscriptions</html>")],
        ),
        (
            "GET /token_dispenser.json".to_string(),
            vec![
                Response::json(TOKEN_JSON),
                Response::json(r#"{"token":"fresh-token"}"#),
            ],
        ),
        (
            "DELETE /users/logout".to_string(),
            vec![Response::redirect("/auth_error")],
        ),
    ]);
    let mut session = active_session(&server, "someone").await;

    let err = session
        .logout()
        .await
        .expect_err("two failures are reported");
    assert!(matches!(err, Error::LogoutFailed(_)));
    assert_eq!(session.state(), SessionState::LoggedOut);
    assert!(session.jar().is_empty());
}

#[tokio::test]
async fn blocked_login_sets_blocked_state() {
    let server = Server::start(vec![("GET /users/login", Response::new(403))]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .login("someone", SecretString::new("hunter2"))
        .await
        .expect_err("blocked");
    assert!(matches!(err, Error::BlockedByEdge { status: 403 }));
    assert_eq!(session.state(), SessionState::Blocked);
}

#[tokio::test]
async fn rate_limited_login_reports_retry_after() {
    let server = Server::start(vec![
        ("GET /users/login", Response::ok(LOGIN_HTML)),
        (
            "POST /users/login",
            Response::new(429).with_header("retry-after", "60"),
        ),
    ]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .login("someone", SecretString::new("hunter2"))
        .await
        .expect_err("throttled");
    assert!(matches!(
        err,
        Error::RateLimited {
            retry_after: Some(60)
        }
    ));
}

#[tokio::test]
async fn login_page_without_csrf_is_drift() {
    let server = Server::start(vec![(
        "GET /users/login",
        Response::ok(LOGIN_HTML_NO_TOKEN),
    )]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .login("someone", SecretString::new("hunter2"))
        .await
        .expect_err("no token");
    assert!(matches!(err, Error::CsrfMissing));
}

#[tokio::test]
async fn token_dispenser_drift_leaves_the_session_unusable() {
    let server = Server::start(vec![
        (
            "GET /users/someone/subscriptions",
            Response::ok("<html>subscriptions</html>"),
        ),
        ("GET /token_dispenser.json", Response::ok("not json")),
    ]);
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .import("someone", complete_jar())
        .await
        .expect_err("bad token json");
    assert!(matches!(err, Error::UpstreamChanged(_)));
    assert_eq!(session.state(), SessionState::Imported);
    assert!(!session.state().is_usable());
    assert!(session.csrf_token().is_err());
}

#[tokio::test]
async fn killed_is_terminal_for_every_network_method() {
    let server = Server::start(probe_routes("someone"));
    let mut session = active_session(&server, "someone").await;
    let requests_before_kill = server.requests().len();

    session.kill();
    assert_eq!(session.state(), SessionState::Killed);
    assert!(session.jar().is_empty());
    assert!(session.csrf_token().is_err());

    assert!(matches!(
        session.import("someone", complete_jar()).await,
        Err(Error::Killed)
    ));
    assert!(matches!(session.probe().await, Err(Error::Killed)));
    assert!(matches!(session.logout().await, Err(Error::Killed)));
    assert!(matches!(
        session.login("someone", SecretString::new("x")).await,
        Err(Error::Killed)
    ));
    assert_eq!(session.state(), SessionState::Killed);
    assert_eq!(
        server.requests().len(),
        requests_before_kill,
        "no network may happen after the kill switch"
    );
}

#[tokio::test]
async fn login_wipes_a_previous_jar_before_authenticating() {
    let mut routes = probe_routes("someone");
    routes.push(("GET /users/login".to_string(), Response::ok(LOGIN_HTML)));
    routes.push(("POST /users/login".to_string(), Response::ok(REJECT_HTML)));
    let server = Server::start(routes);
    let mut session = active_session(&server, "someone").await;
    assert!(session.jar().is_complete());

    let err = session
        .login("other", SecretString::new("wrong"))
        .await
        .expect_err("rejected");
    assert!(matches!(err, Error::LoginRejected(_)));
    assert_eq!(session.account(), "other");
    assert!(
        session.jar().is_empty(),
        "previous identity's jar must be gone"
    );
}

#[tokio::test]
async fn empty_account_is_a_configuration_error() {
    let server = Server::empty();
    let mut session = AccountSession::new(&config_for(&server)).expect("session");

    let err = session
        .import("   ", complete_jar())
        .await
        .expect_err("empty account");
    assert!(matches!(err, Error::Config(_)));
}
