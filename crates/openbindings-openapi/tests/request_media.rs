//! Request media: a supplied body is sent as the operation's one declared media type, or
//! the one JSON type among several, counting concrete JSON keys only; a declared range
//! resolves to `application/json` only as the one declared key or when the caller chooses
//! it; otherwise the caller names a declared key at `<kind>.requestMedia`. In the 3.0 kind
//! a GET, HEAD or DELETE request body has no meaning; in the 3.1 kind it is sent when
//! declared.
#[path = "support/wire.rs"]
mod wire;
use openbindings::Document;
use openbindings_interface_synthesis::{
    CoverageStatus, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_openapi::{
    KIND_30, KIND_31, OpenApi, REQUEST_MEDIA_POINT_30, REQUEST_MEDIA_POINT_31,
};
use serde_json_compat::{Value, json};
use wire::{Seen, Wire, call, challenge, host, source};

const KINDS: [(&str, &str, &str); 2] = [
    (KIND_31, "3.1.2", REQUEST_MEDIA_POINT_31),
    (KIND_30, "3.0.4", REQUEST_MEDIA_POINT_30),
];

fn document(version: &str) -> Value {
    let post = |keys: &[&str]| {
        let content: serde_json_compat::Map<String, Value> =
            keys.iter().map(|k| ((*k).to_owned(), json!({}))).collect();
        json!({"post": {
            "requestBody": {"content": content},
            "responses": {"204": {"description": "empty"}}
        }})
    };
    json!({
        "openapi": version,
        "info": {"title": "Request media", "version": "1"},
        "servers": [{"url": "https://media.example.test"}],
        "paths": {
            "/json": post(&["application/json"]),
            "/vendor": post(&["application/vnd.acme+json"]),
            "/parameterized": post(&["application/json; charset=utf-8"]),
            "/any": post(&["*/*"]),
            "/application": post(&["application/*"]),
            "/among-others": post(&["text/plain", "application/json", "application/xml"]),
            "/json-and-range": post(&["application/json", "*/*"]),
            "/vendor-and-range": post(&["application/vnd.acme+json", "*/*"]),
            "/json-spellings": post(&["application/json", "Application/JSON", "text/plain"]),
            "/text-and-range": post(&["text/plain", "*/*"]),
            "/xml-and-range": post(&["application/xml", "application/*"]),
            "/two-ranges": post(&["*/*", "application/*"]),
            "/two-json": post(&["application/json", "application/merge-patch+json", "text/plain"]),
            "/text-range": post(&["text/*"]),
            "/text": post(&["text/plain", "application/xml"])
        }
    })
}
fn choose(point: &str, key: Value) -> Value {
    json!({"configuration": {point: key}})
}
async fn send(
    src: &openbindings::Source,
    path: &str,
    method: &str,
    input: &str,
    context: Option<&Value>,
) -> Result<Wire, openbindings_invocation::InvocationError> {
    let seen = Seen::default();
    let provider = OpenApi::default().with_transport(host(seen.clone()));
    let result = call(&provider, src, path, method, &[input], context).await;
    let seen = seen.borrow();
    match result {
        Ok(()) => Ok(seen[0].clone()),
        Err(error) => {
            assert!(seen.is_empty(), "a refusal or challenge precedes the host");
            Err(error)
        }
    }
}

#[tokio::test]
async fn the_declared_media_decide_when_they_can() {
    for (kind, version, _) in KINDS {
        let src = source(kind, &document(version), None);
        for (path, media) in [
            ("/json", "application/json"),
            ("/vendor", "application/vnd.acme+json"),
            ("/parameterized", "application/json; charset=utf-8"),
            ("/any", "application/json"),
            ("/application", "application/json"),
            ("/among-others", "application/json"),
            ("/json-and-range", "application/json"),
            // Only concrete JSON keys count.
            ("/vendor-and-range", "application/vnd.acme+json"),
        ] {
            let wire = send(&src, path, "post", r#"{"body":{"n":1.50}}"#, None)
                .await
                .unwrap();
            assert_eq!(wire.header("content-type"), Some(media), "{version} {path}");
            assert_eq!(wire.body.as_deref(), Some(r#"{"n":1.50}"#));
        }
    }
}

#[tokio::test]
async fn otherwise_the_caller_names_a_declared_key() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        for context in [None, Some(json!({"configuration": {}}))] {
            let error = send(
                &src,
                "/two-json",
                "post",
                r#"{"body":{}}"#,
                context.as_ref(),
            )
            .await
            .unwrap_err();
            let challenge = challenge(&error);
            assert_eq!(
                challenge["alternatives"],
                json!([{"requirements": [{
                    "type": "config.value",
                    "point": point,
                    "path": "",
                    "schema": {"type": "string", "enum": ["application/json", "application/merge-patch+json"]}
                }]}]),
                "{version}"
            );
            assert!(challenge["target"].as_str().unwrap().contains("/two-json"));
        }
        for key in ["application/merge-patch+json", "application/json"] {
            let wire = send(
                &src,
                "/two-json",
                "post",
                r#"{"body":{}}"#,
                Some(&choose(point, json!(key))),
            )
            .await
            .unwrap();
            assert_eq!(wire.header("content-type"), Some(key));
        }
        // A choice names a declared key exactly; a declared non-JSON key is not sent.
        for (path, key) in [
            ("/two-json", json!("text/plain")),
            ("/two-json", json!("application/problem+json")),
            ("/two-json", json!("APPLICATION/JSON")),
            ("/two-json", json!(["application/json"])),
            ("/two-json", json!({"key": "application/json"})),
            ("/among-others", json!("text/plain")),
            ("/json", json!("application/xml")),
        ] {
            assert_eq!(
                send(
                    &src,
                    path,
                    "post",
                    r#"{"body":{}}"#,
                    Some(&choose(point, key.clone()))
                )
                .await
                .unwrap_err()
                .code(),
                "ERR_REFUSED",
                "{version} {path} {key}"
            );
        }
        // A choice that agrees with the declarations changes nothing.
        let wire = send(
            &src,
            "/among-others",
            "post",
            r#"{"body":{}}"#,
            Some(&choose(point, json!("application/json"))),
        )
        .await
        .unwrap();
        assert_eq!(wire.header("content-type"), Some("application/json"));
        // Without a body there is no media to choose.
        let wire = send(&src, "/two-json", "post", "{}", None).await.unwrap();
        assert_eq!(wire.header("content-type"), None);
        assert_eq!(wire.body, None);
    }
}

#[tokio::test]
async fn a_range_beside_other_keys_is_sent_only_when_chosen() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        for (path, choices) in [
            ("/text-and-range", &["*/*"][..]),
            ("/xml-and-range", &["application/*"]),
            ("/two-ranges", &["*/*", "application/*"]),
        ] {
            // No JSON is sent unasked: the caller chooses among the keys that can be sent.
            let error = send(&src, path, "post", r#"{"body":{"n":1}}"#, None)
                .await
                .unwrap_err();
            assert_eq!(
                challenge(&error)["alternatives"],
                json!([{"requirements": [{
                    "type": "config.value",
                    "point": point,
                    "path": "",
                    "schema": {"type": "string", "enum": choices}
                }]}]),
                "{version} {path}"
            );
            // Chosen, a range is sent as `application/json`.
            for key in choices {
                let wire = send(
                    &src,
                    path,
                    "post",
                    r#"{"body":{"n":1}}"#,
                    Some(&choose(point, json!(key))),
                )
                .await
                .unwrap();
                assert_eq!(wire.header("content-type"), Some("application/json"));
                assert_eq!(wire.body.as_deref(), Some(r#"{"n":1}"#));
            }
        }
        // A declared key that is not JSON stays refused when chosen.
        for (path, key) in [
            ("/text-and-range", "text/plain"),
            ("/xml-and-range", "application/xml"),
        ] {
            assert_eq!(
                send(
                    &src,
                    path,
                    "post",
                    r#"{"body":{}}"#,
                    Some(&choose(point, json!(key)))
                )
                .await
                .unwrap_err()
                .code(),
                "ERR_REFUSED",
                "{version} {path}"
            );
        }
    }
}

#[tokio::test]
async fn one_json_type_spelled_twice_counts_once() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        // Counted once, the two spellings ask for no choice. The client matches both
        // declarations alike, so the request refuses before it is sent, and so does either
        // spelling chosen.
        for context in [
            None,
            Some(choose(point, json!("application/json"))),
            Some(choose(point, json!("Application/JSON"))),
        ] {
            assert_eq!(
                send(
                    &src,
                    "/json-spellings",
                    "post",
                    r#"{"body":{}}"#,
                    context.as_ref()
                )
                .await
                .unwrap_err()
                .code(),
                "ERR_REFUSED",
                "{version} {context:?}"
            );
        }
    }
}

#[tokio::test]
async fn request_bodies_without_json_media_are_implementation_unsupported() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        let sources = [SynthesisSource {
            name: None,
            source: &src,
        }];
        let result = Synthesizer::new(OpenApi::default())
            .synthesize_with_coverage(&SynthesisRequest {
                sources: &sources,
                ..Default::default()
            })
            .await
            .unwrap();
        for path in ["/text-range", "/text"] {
            let reference = format!("/paths/{}/post", path.replace('/', "~1"));
            assert!(
                result
                    .coverage()
                    .entries()
                    .iter()
                    .any(|e| e.source_ref == reference
                        && e.status == CoverageStatus::ImplementationUnsupported
                        && e.reason_code.as_deref()
                            == Some("openbindings.openapi.request-media-unimplemented")),
                "{version} {path}"
            );
            for context in [None, Some(choose(point, json!("text/plain")))] {
                assert_eq!(
                    send(&src, path, "post", r#"{"body":"x"}"#, context.as_ref())
                        .await
                        .unwrap_err()
                        .code(),
                    "ERR_REFUSED"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_get_or_delete_request_body_has_meaning_only_in_the_31_kind() {
    for (kind, version, _) in KINDS {
        let body = json!({"content": {"application/json": {}}});
        let empty = json!({"204": {"description": "empty"}});
        let document = json!({
            "openapi": version,
            "info": {"title": "Bodies", "version": "1"},
            "servers": [{"url": "https://bodies.example.test"}],
            "paths": {"/search": {
                "get": {"requestBody": body, "responses": empty},
                "delete": {"requestBody": body, "responses": empty}
            }}
        });
        let src = source(kind, &document, None);
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
        let synthesized = Document::parse(synthesized.as_document().value().text()).unwrap();
        let meaningful = kind == KIND_31;
        for operation in synthesized.operations().unwrap() {
            let input: Value =
                serde_json_compat::from_str(operation.input().unwrap().text()).unwrap();
            assert_eq!(
                input["properties"].get("body").is_some(),
                meaningful,
                "{version} {}",
                operation.key()
            );
        }
        for method in ["get", "delete"] {
            let sent = send(&src, "/search", method, r#"{"body":{"q":"x"}}"#, None).await;
            if meaningful {
                let wire = sent.unwrap();
                assert_eq!(wire.body.as_deref(), Some(r#"{"q":"x"}"#));
                assert_eq!(wire.header("content-type"), Some("application/json"));
            } else {
                assert_eq!(
                    sent.unwrap_err().code(),
                    "ERR_REFUSED",
                    "{version} {method}"
                );
            }
            // Without a body, both kinds send none.
            let wire = send(&src, "/search", method, "{}", None).await.unwrap();
            assert_eq!(wire.body, None);
            assert_eq!(wire.header("content-type"), None);
        }
    }
}
