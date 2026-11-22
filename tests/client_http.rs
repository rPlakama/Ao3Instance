#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! HTTP boundary tests: request shape, cookie handling, no hidden redirects.

mod support;

use std::time::Duration;

use ao3instance::ao3::client::Client;
use ao3instance::consts;
use ao3instance::error::Error;
use support::{Response, Server};

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
