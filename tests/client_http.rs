#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! HTTP boundary tests: request shape, cookie handling, no hidden redirects.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ao3instance::ao3::client::Client;
use ao3instance::consts;
use ao3instance::diag::{HttpErrorEvent, format_line};
use ao3instance::error::Error;
use support::{Response, Server};

fn collector() -> (
    Arc<Mutex<Vec<HttpErrorEvent>>>,
    ao3instance::ao3::client::ErrorSink,
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink_events = Arc::clone(&events);
    let sink: ao3instance::ao3::client::ErrorSink = Arc::new(move |event: &HttpErrorEvent| {
        sink_events.lock().expect("event lock").push(event.clone());
    });
    (events, sink)
}

fn client(server: &Server) -> Client {
    // Spacing is a production policy; tests must not wait 2 s per request.
    Client::with_spacing(&server.url(), "AO3Instance/test", Duration::ZERO).expect("client")
}

#[tokio::test]
async fn get_returns_body_and_applies_cookies() {
    let server = Server::start(vec![
        (
            "GET /hello",
            Response::ok("hi").with_cookie("a=1; Path=/; HttpOnly"),
        ),
        ("GET /second", Response::ok("again")),
    ]);
    let mut client = client(&server);

    let response = client.get("/hello").await.expect("first get");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "hi");
    assert!(response.jar_changed);
    assert!(client.jar().contains("a"));

    let second = client.get("/second").await.expect("second get");
    assert!(!second.jar_changed, "no Set-Cookie means no jar change");
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].header("cookie"), Some("a=1"));
    assert_eq!(requests[0].header("user-agent"), Some("AO3Instance/test"));
}

#[tokio::test]
async fn post_form_sends_urlencoded_body_and_captures_cookies() {
    let server = Server::start(vec![(
        "POST /users/login",
        Response::new(302)
            .with_header("location", "/users/someone")
            .with_cookie("_otwarchive_session=s; Path=/")
            .with_cookie("user_credentials=1; Path=/"),
    )]);
    let mut client = client(&server);

    let response = client
        .post_form(
            "/users/login",
            &[("user[login]", "a b"), ("user[password]", "p+w")],
        )
        .await
        .expect("post");
    assert_eq!(response.status, 302);
    assert_eq!(response.location.as_deref(), Some("/users/someone"));
    assert!(client.jar().is_complete());

    let requests = server.requests();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        requests[0].body,
        "user%5Blogin%5D=a+b&user%5Bpassword%5D=p%2Bw"
    );
}

#[tokio::test]
async fn redirects_are_reported_not_followed() {
    let server = Server::start(vec![
        ("GET /go", Response::redirect("/next")),
        ("GET /next", Response::ok("followed")),
    ]);
    let mut client = client(&server);

    let response = client.get("/go").await.expect("get");
    assert_eq!(response.status, 302);
    assert!(response.is_redirect());
    assert_eq!(response.location.as_deref(), Some("/next"));
    assert_eq!(server.requests().len(), 1, "redirect must not be followed");
}

#[tokio::test]
async fn error_statuses_are_data_not_transport_errors() {
    let server = Server::start(vec![
        ("GET /missing", Response::new(404)),
        (
            "GET /throttled",
            Response::new(429).with_header("retry-after", "120"),
        ),
    ]);
    let mut client = client(&server);

    let missing = client.get("/missing").await.expect("404 is a response");
    assert_eq!(missing.status, 404);

    let throttled = client.get("/throttled").await.expect("429 is a response");
    assert_eq!(throttled.status, 429);
    assert_eq!(throttled.retry_after, Some(120));
}

#[tokio::test]
async fn oversized_announced_body_is_refused_before_download() {
    // Announce a body over the cap while writing almost nothing: the client must
    // reject on Content-Length before reading, and the server must not wedge.
    let server = Server::start(vec![(
        "GET /big",
        Response::ok("tiny")
            .with_header("content-length", (consts::MAX_BODY_BYTES + 1).to_string()),
    )]);
    let mut client = client(&server);

    let err = client.get("/big").await.expect_err("cap must trip");
    assert!(matches!(err, Error::UpstreamChanged(_)));
}

#[tokio::test]
async fn delete_with_csrf_sends_token_two_ways() {
    let server = Server::start(vec![(
        "DELETE /users/logout",
        Response::new(302).with_header("location", "/"),
    )]);
    let mut client = client(&server);

    let response = client
        .delete_with_csrf("/users/logout", "tok+/=")
        .await
        .expect("delete");
    assert_eq!(response.status, 302);

    let request = &server.requests()[0];
    assert_eq!(request.method, "DELETE");
    assert_eq!(request.header("x-csrf-token"), Some("tok+/="));
    assert_eq!(request.body, "authenticity_token=tok%2B%2F%3D");
}

#[tokio::test]
async fn timeout_override_aborts_a_slow_response() {
    let server = Server::start(vec![(
        "GET /slow",
        Response::ok("too late").with_delay(Duration::from_millis(300)),
    )]);
    let mut client = client(&server);

    let err = client
        .get_with_timeout("/slow", Duration::from_millis(20))
        .await
        .expect_err("a 20 ms budget must abort a 300 ms response");
    assert!(matches!(err, Error::Network(_)), "got {err:?}");
}

#[tokio::test]
async fn edge_block_emits_one_redacted_diagnostic_event() {
    let server = Server::start(vec![(
        "GET /works/42?view_full_work=true",
        Response::new(403)
            .with_header("cf-ray", "8f2a-SJC")
            .with_header("server", "cloudflare")
            .with_header("content-type", "text/html")
            .with_body("challenge page leaks _otwarchive_session=secret-cookie"),
    )]);
    let mut client = client(&server);
    let (events, sink) = collector();
    client.set_error_sink(sink);

    let response = client
        .get("/works/42?view_full_work=true")
        .await
        .expect("403 is still a response");
    assert_eq!(response.status, 403);
    assert_eq!(response.transport, ao3instance::diag::Transport::Direct);
    assert_eq!(response.path, "/works/42?view_full_work=true");

    let events = events.lock().expect("event lock");
    assert_eq!(events.len(), 1, "one request, one event: {events:?}");
    let event = &events[0];
    assert_eq!(event.status, Some(403));
    assert_eq!(event.error_kind, "blocked_by_edge");
    assert_eq!(event.path, "/works/42?view_full_work=true");
    assert_eq!(event.cf_ray.as_deref(), Some("8f2a-SJC"));
    assert_eq!(event.server_header.as_deref(), Some("cloudflare"));
    assert!(event.latency_ms.is_some());
    assert!(
        !event.message.contains("secret-cookie"),
        "{:?}",
        event.message
    );
    assert!(!format_line(event).contains("secret-cookie"));
}

#[tokio::test]
async fn login_redirect_emits_its_own_kind_without_the_query() {
    let server = Server::start(vec![(
        "GET /works/9",
        Response::redirect("/users/login?return_to=/works/9&authenticity_token=tok"),
    )]);
    let mut client = client(&server);
    let (events, sink) = collector();
    client.set_error_sink(sink);

    let response = client.get("/works/9").await.expect("redirect is data");
    assert_eq!(response.status, 302);

    let events = events.lock().expect("event lock");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].error_kind, "redirect_login");
    assert_eq!(events[0].location_path.as_deref(), Some("/users/login"));
    assert!(!format_line(&events[0]).contains("authenticity_token"));
}

#[tokio::test]
async fn announced_oversize_emits_body_cap_event_with_status() {
    let server = Server::start(vec![(
        "GET /big",
        Response::ok("tiny")
            .with_header("content-length", (consts::MAX_BODY_BYTES + 1).to_string()),
    )]);
    let mut client = client(&server);
    let (events, sink) = collector();
    client.set_error_sink(sink);

    let err = client.get("/big").await.expect_err("cap must trip");
    assert!(matches!(err, Error::UpstreamChanged(_)));

    let events = events.lock().expect("event lock");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].error_kind, "body_cap_exceeded");
    assert_eq!(events[0].status, Some(200));
}

#[tokio::test]
async fn transport_failure_emits_network_event_without_status() {
    let mut client = Client::with_spacing("http://127.0.0.1:1", "AO3Instance/test", Duration::ZERO)
        .expect("client");
    let (events, sink) = collector();
    client.set_error_sink(sink);

    let err = client
        .get("/works/1")
        .await
        .expect_err("connection refused");
    assert!(matches!(err, Error::Network(_)), "got {err:?}");

    let events = events.lock().expect("event lock");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].error_kind, "network");
    assert_eq!(events[0].status, None);
    assert_eq!(events[0].method, "GET");
    assert!(
        events[0].message.contains("connect"),
        "{:?}",
        events[0].message
    );
}
