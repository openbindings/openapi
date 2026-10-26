//! Context admission: whenever the adapter reads the context, at `<kind>.server`,
//! `<kind>.requestMedia` or `<kind>.security`, a context with a member name repeated at
//! any depth, or one over `Options.context_bytes`, refuses before any request. Secured
//! and unsecured operations agree. Every invocation reads a supplied context, since
//! `<kind>.server` applies even with one server; without a context nothing is checked.
#[path = "support/wire.rs"]
mod wire;
use openbindings::{JsonValue, Source};
use openbindings_invocation::{BindingRequest, BindingValue, InvocationError, binding};
use openbindings_openapi::{
    KIND_30, KIND_31, OpenApi, Options, REQUEST_MEDIA_POINT_30, REQUEST_MEDIA_POINT_31,
    SECURITY_POINT_30, SECURITY_POINT_31, SERVER_POINT_30, SERVER_POINT_31,
};
use serde_json_compat::{Value, json};
use wire::{Seen, Wire, host, source, value};

/// A kind with its version and its server, request media and security points.
struct Case {
    kind: &'static str,
    version: &'static str,
    server: &'static str,
    media: &'static str,
    security: &'static str,
}
const CASES: [Case; 2] = [
    Case {
        kind: KIND_31,
        version: "3.1.2",
        server: SERVER_POINT_31,
        media: REQUEST_MEDIA_POINT_31,
        security: SECURITY_POINT_31,
    },
    Case {
        kind: KIND_30,
        version: "3.0.4",
        server: SERVER_POINT_30,
        media: REQUEST_MEDIA_POINT_30,
        security: SECURITY_POINT_30,
    },
];
const A: &str = "https://a.example.test";
const B: &str = "https://b.example.test";
const ONLY: &str = "https://only.example.test";

/// `/open` and `/secured` share two servers; `/one` and `/body` have one server each,
/// and `/body` declares two JSON media.
fn document(version: &str) -> Value {
    let empty = json!({"204": {"description": "empty"}});
    json!({
        "openapi": version,
        "info": {"title": "Context admission", "version": "1"},
        "servers": [{"url": A}, {"url": B}],
        "components": {"securitySchemes": {"Token": {"type": "http", "scheme": "bearer"}}},
        "paths": {
            "/open": {"get": {"responses": empty}},
            "/secured": {"get": {"security": [{"Token": []}], "responses": empty}},
            "/one": {"get": {"servers": [{"url": ONLY}], "responses": empty}},
            "/body": {"post": {
                "servers": [{"url": ONLY}],
                "requestBody": {"content": {
                    "application/json": {},
                    "application/merge-patch+json": {}
                }},
                "responses": empty
            }}
        }
    })
}

/// Invoke one binding with one input value and a context given as JSON text, which may
/// repeat member names. Returns what reached the host.
async fn send(
    options: Options,
    src: &Source,
    path: &str,
    method: &str,
    input: &str,
    context: Option<&str>,
) -> Result<Vec<Wire>, InvocationError> {
    let seen = Seen::default();
    let provider = OpenApi::new(options)
        .unwrap()
        .with_transport(host(seen.clone()));
    let binding = BindingValue {
        content: Some(value(&json!({"path": path, "method": method}))),
        ..Default::default()
    };
    let context = context.map(|text| JsonValue::parse(text).unwrap());
    let invoker = binding::Invoker::new(&provider);
    let mut job = invoker.invoke(BindingRequest {
        source: src,
        binding: &binding,
        context: context.as_ref(),
    });
    let _ = job.write(JsonValue::parse(input).unwrap()).await;
    job.close();
    let outcome = match job.next().await {
        Some(Ok(_)) => panic!("an empty response emits no value"),
        Some(Err(error)) => Err(error),
        None => job.outcome().expect("terminal"),
    };
    let seen = seen.borrow().clone();
    match outcome {
        Ok(()) => Ok(seen),
        Err(error) => {
            assert!(seen.is_empty(), "a refusal or challenge precedes the host");
            Err(error)
        }
    }
}
async fn refused(src: &Source, path: &str, method: &str, input: &str, context: &str) {
    let error = send(Options::default(), src, path, method, input, Some(context))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_REFUSED", "{path} {context}");
}
/// What `/secured` needs to send to `A`: the server, a credential and its grant.
fn secured(case: &Case) -> String {
    json!({
        "credentials": {"Token": "fixture-token"},
        "configuration": {
            case.server: {"url": A},
            case.security: {"origins": {"Token": [A]}}
        }
    })
    .to_string()
}

#[tokio::test]
async fn a_repeated_server_point_refuses() {
    for case in CASES {
        let src = source(case.kind, &document(case.version), None);
        // Read first-wins, either name would choose a destination.
        for (first, second) in [(A, B), (B, A)] {
            let context = format!(
                r#"{{"configuration":{{"{point}":{{"url":"{first}"}},"{point}":{{"url":"{second}"}}}}}}"#,
                point = case.server
            );
            refused(&src, "/open", "get", "{}", &context).await;
        }
        // With one server too: its variables are read from the same point.
        let context = format!(
            r#"{{"configuration":{{"{point}":{{}},"{point}":{{}}}}}}"#,
            point = case.server
        );
        refused(&src, "/one", "get", "{}", &context).await;
        // Named once, the point decides.
        let context = json!({"configuration": {case.server: {"url": B}}}).to_string();
        let wires = send(
            Options::default(),
            &src,
            "/open",
            "get",
            "{}",
            Some(&context),
        )
        .await
        .unwrap();
        assert_eq!(wires[0].target, format!("{B}/open"));
    }
}

#[tokio::test]
async fn a_repeated_request_media_point_refuses() {
    for case in CASES {
        let src = source(case.kind, &document(case.version), None);
        let context = format!(
            r#"{{"configuration":{{"{point}":"application/json","{point}":"application/merge-patch+json"}}}}"#,
            point = case.media
        );
        refused(&src, "/body", "post", r#"{"body":{}}"#, &context).await;
        let context =
            json!({"configuration": {case.media: "application/merge-patch+json"}}).to_string();
        let wires = send(
            Options::default(),
            &src,
            "/body",
            "post",
            r#"{"body":{}}"#,
            Some(&context),
        )
        .await
        .unwrap();
        assert_eq!(
            wires[0].header("content-type"),
            Some("application/merge-patch+json")
        );
    }
}

#[tokio::test]
async fn a_repeated_security_point_refuses() {
    for case in CASES {
        let src = source(case.kind, &document(case.version), None);
        let grant = json!({"origins": {"Token": [A]}}).to_string();
        let context = format!(
            r#"{{"credentials":{{"Token":"fixture-token"}},"configuration":{{"{server}":{{"url":"{A}"}},"{point}":{grant},"{point}":{grant}}}}}"#,
            server = case.server,
            point = case.security
        );
        refused(&src, "/secured", "get", "{}", &context).await;
        let wires = send(
            Options::default(),
            &src,
            "/secured",
            "get",
            "{}",
            Some(&secured(&case)),
        )
        .await
        .unwrap();
        assert_eq!(
            wires[0].header("authorization"),
            Some("Bearer fixture-token")
        );
    }
}

#[tokio::test]
async fn secured_and_unsecured_operations_agree() {
    for case in CASES {
        let src = source(case.kind, &document(case.version), None);
        let good = secured(&case);
        // Each defect on its own: `configuration` twice, a repeated name in a member no
        // point reads, and a repeated credential.
        let configuration = format!(
            r#"{{"configuration":{{"{server}":{{"url":"{A}"}}}},{rest}"#,
            server = case.server,
            rest = &good[1..]
        );
        let unrelated = format!(r#"{{"x-note":{{"a":1,"a":2}},{}"#, &good[1..]);
        let credential = good.replacen(
            r#""Token":"fixture-token""#,
            r#""Token":"fixture-token","Token":"other-token""#,
            1,
        );
        for context in [&configuration, &unrelated, &credential] {
            for path in ["/open", "/secured", "/one"] {
                refused(&src, path, "get", "{}", context).await;
            }
        }
        // Without the defect both operations send.
        for path in ["/open", "/secured"] {
            let wires = send(Options::default(), &src, path, "get", "{}", Some(&good))
                .await
                .unwrap();
            assert_eq!(wires[0].target, format!("{A}{path}"));
        }
        // The same size limit applies to both, exactly at `Options.context_bytes`.
        for (limit, admitted) in [(good.len(), true), (good.len() - 1, false)] {
            let mut options = Options::default();
            options.context_bytes = limit;
            for path in ["/open", "/secured"] {
                let result = send(options.clone(), &src, path, "get", "{}", Some(&good)).await;
                match admitted {
                    true => assert_eq!(result.unwrap().len(), 1),
                    false => assert_eq!(result.unwrap_err().code(), "ERR_REFUSED"),
                }
            }
        }
    }
}

#[tokio::test]
async fn without_a_context_nothing_is_checked() {
    for case in CASES {
        let src = source(case.kind, &document(case.version), None);
        let mut options = Options::default();
        options.context_bytes = 0;
        let wires = send(options.clone(), &src, "/one", "get", "{}", None)
            .await
            .unwrap();
        assert_eq!(wires[0].target, format!("{ONLY}/one"));
        // An empty context is read, and is over a zero limit.
        assert_eq!(
            send(options, &src, "/one", "get", "{}", Some("{}"))
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
    }
}
