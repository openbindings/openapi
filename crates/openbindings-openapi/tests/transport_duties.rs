use dynamic_openapi_client::{
    Cancellation, Header,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::{JsonValue, Source};
use openbindings_invocation::{BindingRequest, BindingValue, InvocationError, binding};
use openbindings_openapi::{KIND_30, KIND_31, OpenApi, Options, SERVER_POINT_30, SERVER_POINT_31};
use serde_json_compat::json;
use std::{cell::Cell, rc::Rc, sync::Arc};

const KINDS: [(&str, &str, &str); 2] = [
    ("3.1.2", KIND_31, SERVER_POINT_31),
    ("3.0.4", KIND_30, SERVER_POINT_30),
];

fn value(text: &str) -> JsonValue {
    JsonValue::parse(text).unwrap()
}

fn source(version: &str, kind: &str, servers: &[&str]) -> Source {
    let document = json!({
        "openapi": version,
        "info": {"title": "Transport duties", "version": "1"},
        "servers": servers.iter().map(|url| json!({"url": url})).collect::<Vec<_>>(),
        "paths": {"/request": {"post": {
            "requestBody": {"content": {"application/json": {}}},
            "responses": {"204": {"description": "Empty"}}
        }}}
    });
    Source {
        kind: kind.into(),
        content: Some(value(
            &json!({"document": document.to_string()}).to_string(),
        )),
        ..Default::default()
    }
}

fn response(status: u16, headers: Vec<Header>) -> TransportResult {
    let mut response = ResponseInput::new(
        status,
        headers,
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
}

async fn call<P: binding::BindingInvoker>(
    provider: &P,
    source: &Source,
    input: &str,
    context: Option<&JsonValue>,
) -> Result<(), InvocationError> {
    let binding = BindingValue {
        content: Some(value(r#"{"path":"/request","method":"post"}"#)),
        ..Default::default()
    };
    let invoker = binding::Invoker::new(provider);
    let mut job = invoker.invoke(BindingRequest {
        source,
        binding: &binding,
        context,
    });
    let _ = job.write(value(input)).await;
    job.close();
    if let Some(output) = job.next().await {
        output?;
        panic!("this profile produces no output");
    }
    job.outcome().expect("terminal")
}

#[tokio::test]
async fn finite_body_budget_precedes_transport_entry() {
    for (version, kind, _) in KINDS {
        let source = source(version, kind, &["https://api.example.test"]);
        let calls = Rc::new(Cell::new(0));
        let entries = calls.clone();
        let mut options = Options::default();
        options.limits.body_bytes = 4;
        let provider = OpenApi::new(options).unwrap().with_transport(
            move |request: PreparedRequest, cancellation: Cancellation| {
                entries.set(entries.get() + 1);
                assert_eq!(request.body().map(AsRef::as_ref), Some(b"null".as_slice()));
                async move {
                    request
                        .invoke(
                            cancellation,
                            &HostCapabilities::programmable(),
                            |_, _| async { response(204, vec![]) },
                        )
                        .await
                }
            },
        );
        call(&provider, &source, r#"{"body":null}"#, None)
            .await
            .unwrap();
        assert_eq!(calls.get(), 1);
        let error = call(&provider, &source, r#"{"body":"abc"}"#, None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_REFUSED");
        assert_eq!(calls.get(), 1);
    }
}

// A followed redirect is a new transport entry at its `Location`; an https to http
// redirect is never followed, so its `Location` never reaches the host.
#[tokio::test]
async fn a_followed_redirect_is_a_new_transport_entry_and_a_downgrade_is_none() {
    for (version, kind, _) in KINDS {
        for status in [301, 302, 303, 307, 308] {
            for (location, entered) in [
                (
                    "http://127.0.0.1/private",
                    vec!["https://api.example.test/request"],
                ),
                (
                    "/moved",
                    vec![
                        "https://api.example.test/request",
                        "https://api.example.test/moved",
                    ],
                ),
            ] {
                let source = source(version, kind, &["https://api.example.test"]);
                let targets = Rc::new(std::cell::RefCell::new(Vec::new()));
                let entries = targets.clone();
                let provider = OpenApi::default().with_transport(
                    move |request: PreparedRequest, cancellation: Cancellation| {
                        entries.borrow_mut().push(request.target().to_owned());
                        let first = entries.borrow().len() == 1;
                        async move {
                            request
                                .invoke(
                                    cancellation,
                                    &HostCapabilities::programmable(),
                                    |_, _| async move {
                                        if first {
                                            response(
                                                status,
                                                vec![Header::new("location", location)],
                                            )
                                        } else {
                                            response(204, vec![])
                                        }
                                    },
                                )
                                .await
                        }
                    },
                );
                let result = call(&provider, &source, "{}", None).await;
                if entered.len() == 1 {
                    // The 3xx is the final response, and no adaptation maps a non-2xx.
                    let error = result.unwrap_err();
                    assert_eq!(error.code(), "ERR_EXECUTION_FAILED");
                    assert!(error.data().is_none());
                } else {
                    result.unwrap();
                }
                assert_eq!(*targets.borrow(), entered, "{version} {status} {location}");
            }
        }
    }
}

#[tokio::test]
async fn status_and_ambiguous_transport_failures_are_never_replayed() {
    for (version, kind, _) in KINDS {
        for failed_upload in [false, true] {
            let source = source(version, kind, &["https://api.example.test"]);
            let calls = Rc::new(Cell::new(0));
            let entries = calls.clone();
            let provider = OpenApi::default().with_transport(
                move |request: PreparedRequest, cancellation: Cancellation| {
                    entries.set(entries.get() + 1);
                    async move {
                        request
                            .invoke(
                                cancellation,
                                &HostCapabilities::programmable(),
                                |_, _| async move {
                                    if failed_upload {
                                        TransportResult {
                                            upload: UploadState::Incomplete,
                                            response: None,
                                            error_detail: Some("ambiguous partial write".into()),
                                        }
                                    } else {
                                        response(503, vec![])
                                    }
                                },
                            )
                            .await
                    }
                },
            );
            let error = call(&provider, &source, r#"{"body":null}"#, None)
                .await
                .unwrap_err();
            assert_eq!(error.code(), "ERR_EXECUTION_FAILED");
            assert!(error.data().is_none());
            assert_eq!(calls.get(), 1);
        }
    }
}

#[tokio::test]
async fn unsecured_requests_attach_no_unselected_credentials() {
    for (version, kind, _) in KINDS {
        let source = source(version, kind, &["https://api.example.test"]);
        let calls = Rc::new(Cell::new(0));
        let entries = calls.clone();
        let provider = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancellation: Cancellation| {
                entries.set(entries.get() + 1);
                assert!(request.headers().iter().all(|header| {
                    !header.is_sensitive()
                        && ![
                            "authorization",
                            "proxy-authorization",
                            "cookie",
                            "x-api-key",
                        ]
                        .iter()
                        .any(|name| header.name().eq_ignore_ascii_case(name))
                }));
                async move {
                    request
                        .invoke(
                            cancellation,
                            &HostCapabilities::programmable(),
                            |_, _| async { response(204, vec![]) },
                        )
                        .await
                }
            },
        );
        let context = value(
            r#"{"bearerToken":"unused","apiKey":"unused","credentials":{"ServiceToken":"unused"},"apiKeys":{"Key":"unused"}}"#,
        );
        for context in [Some(&context), None] {
            call(&provider, &source, "{}", context).await.unwrap();
        }
        assert_eq!(calls.get(), 2);
    }
}

#[tokio::test]
async fn server_choices_refuse_invalid_values_and_challenge_only_when_absent() {
    for (version, kind, server_point) in KINDS {
        for servers in [
            vec!["https://api.example.test"],
            vec!["https://api.example.test", "https://other.example.test"],
        ] {
            let source = source(version, kind, &servers);
            let calls = Rc::new(Cell::new(0));
            let entries = calls.clone();
            let provider = OpenApi::default().with_transport(
                move |request: PreparedRequest, cancellation: Cancellation| {
                    entries.set(entries.get() + 1);
                    async move {
                        request
                            .invoke(
                                cancellation,
                                &HostCapabilities::programmable(),
                                |_, _| async { response(204, vec![]) },
                            )
                            .await
                    }
                },
            );
            for choice in [
                json!(17),
                json!(null),
                json!(false),
                json!([]),
                json!(""),
                // A URL alone is not the point's value, which is an object.
                json!(servers[0]),
                json!({"url": 17}),
                json!({"url": ""}),
                json!({"url": "https://undeclared.example.test"}),
                json!({"url": servers[0], "extra": true}),
                json!({"url": servers[0], "variables": []}),
                json!({"url": servers[0], "variables": {"undeclared": "x"}}),
                json!({"baseUrl": "https://mine.example.test"}),
            ] {
                let context = value(&json!({"configuration": {server_point: choice}}).to_string());
                let error = call(&provider, &source, "{}", Some(&context))
                    .await
                    .unwrap_err();
                assert_eq!(error.code(), "ERR_REFUSED");
                assert!(error.context_required().is_none());
                assert_eq!(
                    calls.get(),
                    0,
                    "invalid choices must precede transport entry"
                );
            }
            let empty_context = value("{}");
            for context in [None, Some(&empty_context)] {
                let result = call(&provider, &source, "{}", context).await;
                if servers.len() == 1 {
                    result.unwrap();
                } else {
                    let error = result.unwrap_err();
                    assert_eq!(error.code(), "CONTEXT_REQUIRED");
                    assert!(error.context_required().is_some());
                    assert_eq!(calls.get(), 0);
                }
            }
            let before = calls.get();
            let context = value(
                &json!({"configuration": {server_point: {"url": servers.last().unwrap()}}})
                    .to_string(),
            );
            call(&provider, &source, "{}", Some(&context))
                .await
                .unwrap();
            assert_eq!(calls.get(), before + 1);
        }
    }
}
