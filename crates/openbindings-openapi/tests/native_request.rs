//! The native request: with no `input` adaptation, the operation's input value is an
//! object with optional `path`, `query`, `header`, `cookie` and `body` members, and the
//! synthesized input contract describes exactly that shape.
use dynamic_openapi_client::{
    Cancellation, Outcome,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::{Document, JsonValue, Source, ValueOutcome};
use openbindings_interface_synthesis::{SynthesisRequest, SynthesisSource, Synthesizer};
use openbindings_invocation::{BindingRequest, BindingValue, binding};
use openbindings_json_schema_evaluator::DefaultEvaluator;
use openbindings_openapi::{KIND_30, KIND_31, OpenApi};
use serde_json_compat::{Value, json};
use std::{cell::RefCell, collections::BTreeSet, rc::Rc, sync::Arc};

const EDITIONS: [(&str, &str); 2] = [(KIND_31, "3.1.2"), (KIND_30, "3.0.4")];

fn description(version: &str) -> String {
    let empty = json!({"204": {"description": "empty"}});
    json!({
        "openapi": version,
        "info": {"title": "Requests", "version": "1"},
        "servers": [{"url": "https://requests.example.test"}],
        "paths": {
            "/items/{id}": {"put": {
                "parameters": [
                    {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                    {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                    {"name": "q", "in": "query", "required": true, "schema": {"type": "string"}},
                    {"name": "id", "in": "query", "schema": {"type": "boolean"}}
                ],
                "requestBody": {"required": true, "content": {"application/json": {}}},
                "responses": empty
            }},
            "/notes": {
                "get": {"responses": empty},
                "post": {"requestBody": {"content": {"application/json": {}}}, "responses": empty}
            }
        }
    })
    .to_string()
}
fn source(kind: &str, version: &str) -> Source {
    Source {
        kind: kind.into(),
        content: Some(
            JsonValue::parse(json!({"document": description(version)}).to_string()).unwrap(),
        ),
        ..Default::default()
    }
}
#[derive(Debug, PartialEq)]
struct Wire {
    method: String,
    target: String,
    headers: Vec<(String, Vec<u8>)>,
    body: Option<Vec<u8>>,
}
type Seen = Rc<RefCell<Vec<Wire>>>;
fn host(
    seen: Seen,
) -> impl Fn(PreparedRequest, Cancellation) -> std::pin::Pin<Box<dyn Future<Output = Outcome>>> {
    move |request, cancellation| {
        let seen = seen.clone();
        Box::pin(async move {
            request
                .invoke(
                    cancellation,
                    &HostCapabilities::programmable(),
                    move |wire, _| async move {
                        seen.borrow_mut().push(Wire {
                            method: wire.method().as_str().to_owned(),
                            target: wire.target().to_owned(),
                            headers: wire
                                .headers()
                                .iter()
                                .map(|h| (h.name().to_ascii_lowercase(), h.value().to_vec()))
                                .collect(),
                            body: wire.body().map(|b| b.to_vec()),
                        });
                        let mut response = ResponseInput::new(
                            204,
                            vec![],
                            Arc::from([]),
                            BodyState::Complete,
                            Provenance::Wire,
                        );
                        response.header_delivery = HeaderDelivery::Complete;
                        TransportResult {
                            upload: UploadState::Complete,
                            response: Some(response),
                            error_detail: None,
                        }
                    },
                )
                .await
        })
    }
}
async fn call<P: binding::BindingInvoker>(
    provider: P,
    source: &Source,
    path: &str,
    method: &str,
    input: &str,
) -> Result<(), openbindings_invocation::InvocationError> {
    call_with(provider, source, path, method, &[input]).await
}
async fn call_with<P: binding::BindingInvoker>(
    provider: P,
    source: &Source,
    path: &str,
    method: &str,
    inputs: &[&str],
) -> Result<(), openbindings_invocation::InvocationError> {
    let binding = BindingValue {
        content: Some(
            JsonValue::parse(json!({"path": path, "method": method}).to_string()).unwrap(),
        ),
        ..Default::default()
    };
    let invoker = binding::Invoker::new(provider);
    let mut job = invoker.invoke(BindingRequest {
        source,
        binding: &binding,
        context: None,
    });
    for input in inputs {
        let _ = job.write(JsonValue::parse(input).unwrap()).await;
    }
    job.close();
    if let Some(value) = job.next().await {
        value?;
        panic!("an empty response emits no value");
    }
    job.outcome().expect("terminal")
}
fn query_pairs(target: &str) -> BTreeSet<&str> {
    target
        .split_once('?')
        .map_or("", |(_, q)| q)
        .split('&')
        .collect()
}

#[tokio::test]
async fn request_members_reach_their_locations_and_the_body() {
    for (kind, version) in EDITIONS {
        let src = source(kind, version);
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        // Member order is immaterial, and one name may be declared in two locations.
        for input in [
            r#"{"path":{"id":"a b"},"query":{"q":"x","limit":10,"id":true},"body":{"n":9007199254740993}}"#,
            r#"{"body":{"n":9007199254740993},"query":{"id":true,"limit":10,"q":"x"},"path":{"id":"a b"}}"#,
        ] {
            call(&provider, &src, "/items/{id}", "put", input)
                .await
                .unwrap();
        }
        let seen = seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], seen[1]);
        assert_eq!(seen[0].method, "PUT");
        assert!(
            seen[0]
                .target
                .starts_with("https://requests.example.test/items/a%20b?")
        );
        assert_eq!(
            query_pairs(&seen[0].target),
            BTreeSet::from(["id=true", "limit=10", "q=x"])
        );
        assert_eq!(
            seen[0].body.as_deref(),
            Some(br#"{"n":9007199254740993}"#.as_slice())
        );
    }
}

#[tokio::test]
async fn empty_header_and_cookie_members_contribute_nothing() {
    for (kind, version) in EDITIONS {
        let src = source(kind, version);
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        for input in [
            "{}",
            r#"{"header":{}}"#,
            r#"{"cookie":{}}"#,
            r#"{"path":{},"query":{},"header":{},"cookie":{}}"#,
        ] {
            call(&provider, &src, "/notes", "get", input).await.unwrap();
        }
        let seen = seen.borrow();
        assert_eq!(seen.len(), 4);
        assert!(seen.iter().all(|wire| *wire == seen[0]));
        assert_eq!(seen[0].target, "https://requests.example.test/notes");
        assert!(
            seen[0]
                .headers
                .iter()
                .all(|(name, _)| name != "cookie" && !name.starts_with("x-"))
        );
    }
}

#[tokio::test]
async fn header_and_cookie_members_name_no_declared_parameter() {
    for (kind, version) in EDITIONS {
        let src = source(kind, version);
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        for input in [
            r#"{"header":{"X-Trace":"1"}}"#,
            r#"{"header":{"Accept":"application/json"}}"#,
            r#"{"cookie":{"session":"a"}}"#,
            r#"{"header":"X-Trace: 1"}"#,
            r#"{"header":null}"#,
            r#"{"cookie":[]}"#,
        ] {
            assert_eq!(
                call(&provider, &src, "/notes", "get", input)
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_REFUSED",
                "{input}"
            );
        }
        assert!(seen.borrow().is_empty());
    }
}

#[tokio::test]
async fn values_outside_the_native_request_are_refused_before_any_request() {
    for (kind, version) in EDITIONS {
        let src = source(kind, version);
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        let accepted = r#"{"path":{"id":"a"},"query":{"q":"x"},"body":{}}"#;
        call(&provider, &src, "/items/{id}", "put", accepted)
            .await
            .unwrap();
        for input in [
            "null",
            "[]",
            r#""path""#,
            // The earlier `parameters` wrapper is a member this request does not have.
            r#"{"parameters":{"path":{"id":"a"},"query":{"q":"x"}},"body":{}}"#,
            r#"{"path":{"id":"a"},"query":{"q":"x"},"body":{},"extra":1}"#,
            r#"{"path":{"id":"a"},"query":{"q":"x"},"body":{},"headers":{}}"#,
            r#"{"path":"a","query":{"q":"x"},"body":{}}"#,
            r#"{"path":{"id":"a"},"query":[],"body":{}}"#,
            r#"{"path":{"id":"a"},"query":{"q":"x","other":"y"},"body":{}}"#,
            r#"{"path":{"id":"a","limit":1},"query":{"q":"x"},"body":{}}"#,
            r#"{"path":{"id":7},"query":{"q":"x"},"body":{}}"#,
            r#"{"path":{"id":"a"},"query":{"q":"x","limit":1.5},"body":{}}"#,
            r#"{"path":{"id":"a"},"query":{"q":"x","id":"true"},"body":{}}"#,
            // Null parameter values and duplicate names stay refused.
            r#"{"path":{"id":null},"query":{"q":"x"},"body":{}}"#,
            r#"{"path":{"id":"a"},"query":{"q":"x","limit":null},"body":{}}"#,
            r#"{"path":{"id":"a"},"query":{"q":"x","q":"y"},"body":{}}"#,
            r#"{"path":{"id":"a"},"path":{"id":"a"},"query":{"q":"x"},"body":{}}"#,
            r#"{"path":{"id":"a"},"query":{"q":"x"},"body":{"n":1,"n":2}}"#,
            r#"{"path":{"id":"a"},"query":{"q":"x"},"body":{},"body":{}}"#,
        ] {
            assert_eq!(
                call(&provider, &src, "/items/{id}", "put", input)
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_REFUSED",
                "{input}"
            );
        }
        assert_eq!(seen.borrow().len(), 1);
    }
}

#[tokio::test]
async fn input_that_closes_without_a_value_makes_the_request_with_nothing_in_it() {
    for (kind, version) in EDITIONS {
        let src = source(kind, version);
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        call_with(&provider, &src, "/notes", "get", &[])
            .await
            .unwrap();
        // An optional request body is simply absent.
        call_with(&provider, &src, "/notes", "post", &[])
            .await
            .unwrap();
        // The usual checks apply: required parameters and a required body stay required.
        assert_eq!(
            call_with(&provider, &src, "/items/{id}", "put", &[])
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
        // A second value is refused before any request.
        assert_eq!(
            call_with(&provider, &src, "/notes", "get", &["{}", "{}"])
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
        {
            let seen = seen.borrow();
            assert_eq!(seen.len(), 2);
            for (wire, method) in seen.iter().zip(["GET", "POST"]) {
                assert_eq!(wire.method, method);
                assert_eq!(wire.target, "https://requests.example.test/notes");
                assert_eq!(wire.body, None);
                assert!(wire.headers.iter().all(|(name, _)| name != "content-type"));
            }
        }
        // No value and an empty value make the same request.
        call(&provider, &src, "/notes", "post", "{}").await.unwrap();
        let seen = seen.borrow();
        assert_eq!(seen[2], seen[1]);
    }
}

#[tokio::test]
async fn an_absent_body_sends_no_content_and_a_null_body_sends_json_null() {
    for (kind, version) in EDITIONS {
        let src = source(kind, version);
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        call(&provider, &src, "/notes", "post", "{}").await.unwrap();
        call(&provider, &src, "/notes", "post", r#"{"body":null}"#)
            .await
            .unwrap();
        // No request body is declared for this GET, so `body` names nothing it can carry.
        assert_eq!(
            call(&provider, &src, "/notes", "get", r#"{"body":null}"#)
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
        let seen = seen.borrow();
        assert_eq!(seen.len(), 2);
        let media = |wire: &Wire| {
            wire.headers
                .iter()
                .filter(|(name, _)| name == "content-type")
                .map(|(_, value)| value.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(seen[0].body, None);
        assert!(media(&seen[0]).is_empty());
        assert_eq!(seen[1].body.as_deref(), Some(b"null".as_slice()));
        assert_eq!(media(&seen[1]), [b"application/json".to_vec()]);
    }
}

#[tokio::test]
async fn the_synthesized_input_contract_describes_the_native_request() {
    let location = |properties: Value, required: Value| json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false});
    let none = || location(json!({}), json!([]));
    let expected = [
        (
            "/items/{id}",
            "put",
            json!({
                "type": "object",
                "properties": {
                    "path": location(json!({"id": {"type": "string"}}), json!(["id"])),
                    "query": location(
                        json!({"limit": {"type": "integer"}, "q": {"type": "string"}, "id": {"type": "boolean"}}),
                        json!(["q"]),
                    ),
                    "body": true
                },
                "required": ["path", "query", "body"],
                "additionalProperties": false
            }),
        ),
        (
            "/notes",
            "post",
            json!({
                "type": "object",
                "properties": {"path": none(), "query": none(), "body": true},
                "required": [],
                "additionalProperties": false
            }),
        ),
        (
            "/notes",
            "get",
            json!({
                "type": "object",
                "properties": {"path": none(), "query": none()},
                "required": [],
                "additionalProperties": false
            }),
        ),
    ];
    for (kind, version) in EDITIONS {
        let src = source(kind, version);
        let sources = [SynthesisSource {
            name: Some("api"),
            source: &src,
        }];
        let synthesized = Synthesizer::new(OpenApi::default())
            .synthesize(&SynthesisRequest {
                sources: &sources,
                ..Default::default()
            })
            .await
            .unwrap();
        let document = Document::parse(synthesized.as_document().value().text()).unwrap();
        let key = |path: &str, method: &str| {
            document
                .bindings()
                .unwrap()
                .unwrap()
                .into_iter()
                .find(|b| {
                    let content = b.content().unwrap();
                    content.get("path").unwrap().as_str() == Some(path)
                        && content.get("method").unwrap().as_str() == Some(method)
                })
                .unwrap()
                .operation()
                .unwrap()
                .to_owned()
        };
        for (path, method, schema) in &expected {
            let key = key(path, method);
            let operation = document
                .operations()
                .unwrap()
                .into_iter()
                .find(|o| o.key() == key)
                .unwrap();
            let actual: Value =
                serde_json_compat::from_str(operation.input().unwrap().text()).unwrap();
            assert_eq!(&actual, schema, "{version} {method} {path}");
        }
        // The contract's verdicts agree with what the request accepts.
        let put_key = key("/items/{id}", "put");
        let conformant = document.clone().validate().unwrap().conformant().unwrap();
        let put = conformant
            .operations()
            .into_iter()
            .find(|o| o.key() == put_key)
            .unwrap()
            .input()
            .unwrap()
            .prepare(&DefaultEvaluator::new())
            .unwrap();
        for (input, satisfies) in [
            (r#"{"path":{"id":"a"},"query":{"q":"x"},"body":{}}"#, true),
            (
                r#"{"path":{"id":"a"},"query":{"q":"x","limit":2,"id":false},"body":null}"#,
                true,
            ),
            (r#"{"path":{"id":"a"},"body":{}}"#, false),
            (r#"{"path":{},"query":{"q":"x"},"body":{}}"#, false),
            (r#"{"path":{"id":"a"},"query":{"q":"x"}}"#, false),
            (
                r#"{"parameters":{"path":{"id":"a"},"query":{"q":"x"}},"body":{}}"#,
                false,
            ),
            (
                r#"{"path":{"id":"a"},"query":{"q":"x","other":"y"},"body":{}}"#,
                false,
            ),
            // The contract describes only the locations this operation declares.
            (
                r#"{"path":{"id":"a"},"query":{"q":"x"},"header":{},"body":{}}"#,
                false,
            ),
        ] {
            let outcome = put.validate(&JsonValue::parse(input).unwrap());
            assert_eq!(
                matches!(outcome, ValueOutcome::Satisfies),
                satisfies,
                "{version} {input}"
            );
        }
    }
}
