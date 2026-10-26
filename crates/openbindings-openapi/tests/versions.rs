//! Second-edition acceptance: author-owned fixtures and literal wire expectations.
#[path = "support/source.rs"]
mod source_support;
use openbindings::{Document, JsonValue, Source};
use openbindings_interface_synthesis::{
    CoverageStatus, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_openapi::{KIND_30, KIND_31, OpenApi, Options, ResourceInput};
use openbindings_source_inspection::Inspector;
const SIMPLE: &str = r#"{"openapi":"3.0.4","info":{"title":"Version test","version":"1"},"servers":[{"url":"https://version.example.invalid"}],"paths":{"/ping":{"get":{"responses":{"204":{"description":"No entity"}}}}}}"#;
fn json(t: &str) -> JsonValue {
    JsonValue::parse(t).unwrap()
}
fn source(kind: &str, text: &str) -> Source {
    Source {
        kind: kind.into(),
        content: Some(json(
            &serde_json_compat::json!({"document":text}).to_string(),
        )),
        ..Default::default()
    }
}
async fn synth<P: openbindings_interface_synthesis::InterfaceSynthesizer>(
    provider: P,
    src: &Source,
) -> Result<
    openbindings_interface_synthesis::Synthesis,
    openbindings_interface_synthesis::SynthesisError,
> {
    let inputs = [SynthesisSource {
        name: Some("archive"),
        source: src,
    }];
    Synthesizer::new(provider)
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &inputs,
            ..Default::default()
        })
        .await
}
#[tokio::test]
async fn correct_editions_and_kind_sensitive_cache() {
    let provider = OpenApi::default();
    for (kind, version, other) in [(KIND_30, "3.0.4", KIND_31), (KIND_31, "3.1.2", KIND_30)] {
        let src = source(kind, &SIMPLE.replace("3.0.4", version));
        assert_eq!(
            Inspector::new(&provider)
                .inspect(&src)
                .await
                .unwrap()
                .targets
                .len(),
            1
        );
        assert!(
            synth(&provider, &src)
                .await
                .unwrap()
                .coverage()
                .strict_eligible()
        );
        let wrong = Source {
            kind: other.into(),
            ..src.clone()
        }; // Identical JsonValue owner.
        assert!(Inspector::new(&provider).inspect(&wrong).await.is_err());
        assert!(synth(&provider, &wrong).await.is_err());
        assert!(Inspector::new(&provider).inspect(&src).await.is_ok());
    }
}

use dynamic_openapi_client::{
    Cancellation, Header, Outcome,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings_invocation::{BindingRequest, BindingValue, InvocationError, binding, operation};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};
fn transport(
    count: Rc<Cell<usize>>,
    targets: Rc<RefCell<Vec<String>>>,
    status: u16,
    body: &'static str,
    state: BodyState,
) -> impl Fn(PreparedRequest, Cancellation) -> std::pin::Pin<Box<dyn Future<Output = Outcome>>> {
    move |request, cancel| {
        let count = count.clone();
        let targets = targets.clone();
        Box::pin(async move {
            request
                .invoke(
                    cancel,
                    &HostCapabilities::programmable(),
                    move |wire, _| async move {
                        count.set(count.get() + 1);
                        targets.borrow_mut().push(wire.target().to_owned());
                        let mut response = ResponseInput::new(
                            status,
                            vec![Header::new("content-type", "application/json")],
                            Arc::from(body.as_bytes()),
                            state,
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
async fn invoke<P: binding::BindingInvoker>(
    provider: P,
    src: &Source,
    path: &str,
    method: &str,
    inputs: &[&str],
    context: Option<&JsonValue>,
) -> Result<Vec<JsonValue>, InvocationError> {
    let facade = binding::Invoker::new(provider);
    let value = BindingValue {
        content: Some(json(
            &serde_json_compat::json!({"path":path,"method":method}).to_string(),
        )),
        ..Default::default()
    };
    let mut job = facade.invoke(BindingRequest {
        source: src,
        binding: &value,
        context,
    });
    for input in inputs {
        let _ = job.write(json(input)).await;
    }
    job.close();
    let mut values = vec![];
    while let Some(value) = job.next().await {
        match value {
            Ok(value) => values.push(value),
            Err(error) => {
                assert!(values.is_empty(), "failure emitted output");
                return Err(error);
            }
        }
    }
    job.outcome().unwrap()?;
    Ok(values)
}
#[tokio::test]
async fn preflight_makes_no_prediction_without_dispatch() {
    // The adapter keeps the provider's default preflight hook, which declares
    // none; the facade admits the request, finds no hook and predicts nothing,
    // without dispatching.
    let count = Rc::new(Cell::new(0));
    let provider = OpenApi::default().with_transport(transport(
        count.clone(),
        Rc::new(RefCell::new(Vec::new())),
        204,
        "",
        BodyState::Complete,
    ));
    let facade = binding::Invoker::new(&provider);
    let src = source(KIND_30, SIMPLE);
    let value = BindingValue {
        content: Some(json(r#"{"path":"/ping","method":"get"}"#)),
        ..Default::default()
    };
    let request = BindingRequest {
        source: &src,
        binding: &value,
        context: None,
    };
    assert!(facade.preflight(request).await.unwrap().is_none());
    let control = openbindings_source_inspection::CancellationToken::new();
    control.cancel();
    assert_eq!(
        facade
            .preflight_with_control(request, &control)
            .await
            .unwrap_err()
            .code(),
        "ERR_CANCELLED"
    );
    assert_eq!(count.get(), 0);
}

#[tokio::test]
async fn kinds_are_advertised_and_crossed_editions_never_dispatch() {
    // Label and point values are asserted with the retired labels in `kinds.rs`.
    let count = Rc::new(Cell::new(0));
    let provider = OpenApi::default().with_transport(transport(
        count.clone(),
        Default::default(),
        204,
        "",
        BodyState::Complete,
    ));
    let expected = vec![KIND_31.to_owned(), KIND_30.to_owned()];
    assert_eq!(
        openbindings_source_inspection::SourceInspector::supported_kinds(&provider)
            .into_iter()
            .map(|k| k.kind)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        openbindings_interface_synthesis::InterfaceSynthesizer::supported_kinds(&provider)
            .into_iter()
            .map(|k| k.kind)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        binding::BindingInvoker::supported_kinds(&provider)
            .into_iter()
            .map(|k| k.kind)
            .collect::<Vec<_>>(),
        expected
    );
    for (kind, version, other) in [(KIND_30, "3.0.4", KIND_31), (KIND_31, "3.1.2", KIND_30)] {
        let src = source(kind, &SIMPLE.replace("3.0.4", version));
        invoke(&provider, &src, "/ping", "get", &["{}"], None)
            .await
            .unwrap();
        let before = count.get();
        let wrong = Source {
            kind: other.into(),
            ..src.clone()
        };
        assert_eq!(
            invoke(&provider, &wrong, "/ping", "get", &["{}"], None)
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
        assert_eq!(count.get(), before);
        invoke(&provider, &src, "/ping", "get", &["{}"], None)
            .await
            .unwrap();
    }
    for kind in [KIND_31, KIND_30, "unknown.kind"] {
        for version in ["2.0", "3.2.0"] {
            let src = source(kind, &SIMPLE.replace("3.0.4", version));
            let before = count.get();
            assert!(Inspector::new(&provider).inspect(&src).await.is_err());
            assert!(synth(&provider, &src).await.is_err());
            assert!(
                invoke(&provider, &src, "/ping", "get", &["{}"], None)
                    .await
                    .is_err()
            );
            assert_eq!(count.get(), before);
        }
    }
    for version in ["3.0.0", "3.0.3"] {
        let src = source(KIND_30, &SIMPLE.replace("3.0.4", version));
        assert!(
            synth(&provider, &src)
                .await
                .unwrap()
                .coverage()
                .strict_eligible()
        );
        invoke(&provider, &src, "/ping", "get", &["{}"], None)
            .await
            .unwrap();
    }
}
#[tokio::test]
async fn parameter_types_require_a_supported_lexical_schema_dialect() {
    const OLD: &str = "http://json-schema.org/draft-07/schema#";
    const OPAQUE: &str = "https://schemas.example.invalid/custom";
    const OAS: &str = "https://spec.openapis.org/oas/3.1/dialect/base";
    const DATED_OAS: &str = "https://spec.openapis.org/oas/3.1/dialect/2024-11-10";
    const MODERN: &str = "https://json-schema.org/draft/2020-12/schema";
    for (default, local, reference, represented) in [
        (Some(OLD), None, true, false),
        (None, Some(OLD), true, false),
        (Some(OPAQUE), None, false, false),
        (None, Some(OPAQUE), false, false),
        (Some(OAS), None, true, true),
        (None, Some(DATED_OAS), true, true),
        (Some(MODERN), None, true, true),
        (Some(OLD), Some(MODERN), true, true),
        (Some(MODERN), Some(OLD), true, false),
    ] {
        let mut api: serde_json_compat::Value = serde_json_compat::from_str(SIMPLE).unwrap();
        api["openapi"] = "3.1.2".into();
        if let Some(dialect) = default {
            api["jsonSchemaDialect"] = dialect.into();
        }
        api["components"] = serde_json_compat::json!({"schemas":{"Count":{"type":"integer"}}});
        let mut schema = serde_json_compat::json!({"type":"string"});
        if reference {
            schema["$ref"] = "#/components/schemas/Count".into();
        }
        if let Some(dialect) = local {
            schema["$schema"] = dialect.into();
        }
        api["paths"]["/ping"]["get"]["parameters"] = serde_json_compat::json!([
            {"name":"limit","in":"query","schema":schema}
        ]);
        let src = source(KIND_31, &api.to_string());
        let calls = Rc::new(Cell::new(0));
        let targets = Rc::new(RefCell::new(Vec::new()));
        let provider = OpenApi::default().with_transport(transport(
            calls.clone(),
            targets.clone(),
            204,
            "",
            BodyState::Complete,
        ));
        let inventory = Inspector::new(&provider).inspect(&src).await.unwrap();
        assert_eq!(inventory.targets.len(), 1);
        assert_eq!(
            inventory.targets[0].operation.is_some(),
            represented,
            "default={default:?}, local={local:?}"
        );
        let report = synth(&provider, &src).await.unwrap();
        assert_eq!(report.coverage().strict_eligible(), represented);
        let draft = report.document().as_document();
        assert_eq!(draft.operations().unwrap().len(), usize::from(represented));
        assert_eq!(calls.get(), 0, "authoring must not dispatch");
        if represented {
            // Only the explicit modern type is projected; the unresolved reference
            // remains disclosed loss, not an effective schema-validation claim.
            assert!(!report.coverage().fully_represented());
            let input = draft
                .operations()
                .unwrap()
                .remove(0)
                .input()
                .unwrap()
                .to_owned();
            assert_eq!(
                input
                    .get("properties")
                    .unwrap()
                    .get("query")
                    .unwrap()
                    .get("properties")
                    .unwrap()
                    .get("limit")
                    .unwrap()
                    .get("type")
                    .unwrap()
                    .as_str(),
                Some("string")
            );
            invoke(
                &provider,
                &src,
                "/ping",
                "get",
                &[r#"{"query":{"limit":"abc"}}"#],
                None,
            )
            .await
            .unwrap();
            assert_eq!(calls.get(), 1);
            assert_eq!(
                targets.borrow()[0],
                "https://version.example.invalid/ping?limit=abc"
            );
        } else {
            assert!(report.coverage().entries().iter().any(|entry| entry.status
                == CoverageStatus::ImplementationUnsupported
                && entry.reason_code.as_deref()
                    == Some("openbindings.openapi.parameter-schema-dialect-unimplemented")));
            for input in [r#"{"query":{"limit":5}}"#, r#"{"query":{"limit":"abc"}}"#] {
                assert_eq!(
                    invoke(&provider, &src, "/ping", "get", &[input], None)
                        .await
                        .unwrap_err()
                        .code(),
                    "ERR_REFUSED"
                );
            }
            assert_eq!(calls.get(), 0);
        }
    }
}

#[tokio::test]
async fn schema_dialect_boundaries_do_not_invent_a_caller_contract() {
    for (kind, version, schema, represented) in [
        (KIND_30, "3.0.4", r#"{"type":"string"}"#, true),
        (
            KIND_30,
            "3.0.4",
            r#"{"type":"string","nullable":false}"#,
            true,
        ),
        (
            KIND_30,
            "3.0.4",
            r#"{"type":"string","nullable":true}"#,
            false,
        ),
        (KIND_30, "3.0.4", r#"{"nullable":true}"#, false),
        (
            KIND_30,
            "3.0.4",
            r##"{"$ref":"#/components/schemas/Text","type":"integer"}"##,
            false,
        ),
        (
            KIND_31,
            "3.1.2",
            r##"{"$ref":"#/components/schemas/Text","type":"string"}"##,
            true,
        ),
        (KIND_31, "3.1.2", r#"{"type":["string","null"]}"#, false),
        (KIND_31, "3.1.2", "true", false),
    ] {
        let mut api: serde_json_compat::Value = serde_json_compat::from_str(SIMPLE).unwrap();
        api["openapi"] = version.into();
        api["components"] = serde_json_compat::json!({"schemas":{"Text":{"type":"string"}}});
        api["paths"]["/ping"]["get"]["parameters"] = serde_json_compat::json!([{"name":"q","in":"query","schema":serde_json_compat::from_str::<serde_json_compat::Value>(schema).unwrap()}]);
        let src = source(kind, &api.to_string());
        let count = Rc::new(Cell::new(0));
        let provider = OpenApi::default().with_transport(transport(
            count.clone(),
            Default::default(),
            204,
            "",
            BodyState::Complete,
        ));
        let inventory = Inspector::new(&provider).inspect(&src).await.unwrap();
        assert_eq!(inventory.targets.len(), 1);
        let generated = synth(&provider, &src).await.unwrap();
        assert_eq!(
            generated.coverage().strict_eligible(),
            represented,
            "{kind} {schema}"
        );
        assert_eq!(count.get(), 0);
        if !represented {
            assert!(
                generated
                    .coverage()
                    .entries()
                    .iter()
                    .any(|e| e.status == CoverageStatus::ImplementationUnsupported)
            );
            assert!(
                generated
                    .document()
                    .as_document()
                    .clone()
                    .operations()
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                invoke(&provider, &src, "/ping", "get", &["{}"], None)
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_REFUSED"
            );
            assert_eq!(count.get(), 0);
        }
    }
    // Protocol Reference Objects are resolved by the client. Conflicting siblings
    // on 3.0 parameter/requestBody/response references are ignored upstream.
    let api = r##"{"openapi":"3.0.4","info":{"title":"References","version":"1"},"servers":[{"url":"https://version.example.invalid"}],"paths":{"/ping":{"post":{"parameters":[{"$ref":"#/components/parameters/Q","name":"wrong","in":"header","schema":{"type":"array"}}],"requestBody":{"$ref":"#/components/requestBodies/Body","content":{"text/plain":{}}},"responses":{"200":{"$ref":"#/components/responses/Reply","content":{"text/plain":{}}}}}}},"components":{"parameters":{"Q":{"name":"q","in":"query","schema":{"type":"string"}}},"requestBodies":{"Body":{"content":{"application/json":{"schema":{"type":"object","nullable":true}}}}},"responses":{"Reply":{"description":"JSON","content":{"application/json":{"schema":{"type":"object","nullable":true}}}}}}}"##;
    let src = source(KIND_30, api);
    let count = Rc::new(Cell::new(0));
    let provider = OpenApi::default().with_transport(transport(
        count.clone(),
        Default::default(),
        200,
        "null",
        BodyState::Complete,
    ));
    let report = synth(&provider, &src).await.unwrap();
    assert!(report.coverage().strict_eligible());
    assert!(!report.coverage().fully_represented());
    let draft = report.document().as_document();
    let op = draft.operations().unwrap().remove(0);
    assert!(op.output().is_none());
    assert_eq!(
        op.input()
            .unwrap()
            .get("properties")
            .unwrap()
            .get("body")
            .unwrap()
            .text(),
        "true"
    );
    assert_eq!(
        invoke(
            &provider,
            &src,
            "/ping",
            "post",
            &[r#"{"query":{"q":"x"},"body":null}"#],
            None
        )
        .await
        .unwrap()[0]
            .text(),
        "null"
    );
    assert_eq!(count.get(), 1);
}
fn resources() -> Options {
    use dynamic_openapi_client::{graph::ReferenceKind, inspect::Edition};
    use openbindings_openapi::ResourceLayout;
    let mut options = Options::default();
    options.resources = vec![
        ResourceInput::at(
            "https://descriptions.example.invalid/archive/pieces/routes.json",
            include_bytes!("fixtures/v30/routes.json").as_slice(),
        )
        .unwrap()
        .layout(ResourceLayout::new().protocol(
            "/Lookup",
            ReferenceKind::PathItem,
            Edition::OpenApi30,
        )),
        ResourceInput::at(
            "https://descriptions.example.invalid/archive/pieces/parameters.json",
            include_bytes!("fixtures/v30/parameters.json").as_slice(),
        )
        .unwrap()
        .layout(ResourceLayout::new().protocol(
            "/Key",
            ReferenceKind::Parameter,
            Edition::OpenApi30,
        )),
    ];
    options
}
#[tokio::test]
async fn independent_manual_and_synthesized_reloaded_consumers_use_real_http() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    let exchanges: Vec<serde_json_compat::Value> =
        serde_json_compat::from_str(include_str!("fixtures/v30/exchanges.json")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let oracle = exchanges.clone();
    let wire = std::thread::spawn(move || {
        for row in oracle.iter().cycle().take(oracle.len() * 2) {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut socket = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing HTTP exchange");
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut b = [0];
                socket.read_exact(&mut b).unwrap();
                head.push(b[0]);
                assert!(head.len() < 65536);
            }
            let head = String::from_utf8(head).unwrap();
            assert_eq!(
                head.lines().next().unwrap(),
                format!(
                    "{} {} HTTP/1.1",
                    row["method"].as_str().unwrap(),
                    row["target"].as_str().unwrap()
                )
            );
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; length];
            socket.read_exact(&mut body).unwrap();
            assert_eq!(
                body,
                row["request"].as_str().unwrap_or("").as_bytes(),
                "literal request entity bytes"
            );
            let status = row["status"].as_u64().unwrap();
            let response = row["response"].as_str().unwrap();
            let media = if status == 404 {
                "text/plain"
            } else {
                "application/json"
            };
            write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
        }
    });
    let manual = include_str!("fixtures/v30/manual.obi.json")
        .replace("https://archive.example.invalid", &origin);
    let saved = {
        let document = Document::parse(&manual).unwrap();
        let draft = &document;
        let src = &source_support::source(draft.source("archive").unwrap().unwrap());
        let count = Rc::new(Cell::new(0));
        let provider = OpenApi::new(resources()).unwrap().with_transport(transport(
            count.clone(),
            Default::default(),
            200,
            "null",
            BodyState::Complete,
        ));
        assert_eq!(
            Inspector::new(&provider)
                .inspect(src)
                .await
                .unwrap()
                .targets
                .len(),
            2
        );
        let generated = synth(&provider, src).await.unwrap();
        assert!(generated.coverage().strict_eligible());
        assert!(!generated.coverage().fully_represented());
        assert_eq!(count.get(), 0);
        let new = generated.document().as_document();
        assert_eq!(
            new.source("archive")
                .unwrap()
                .unwrap()
                .content()
                .unwrap()
                .text(),
            src.content.as_ref().unwrap().text()
        );
        assert_eq!(
            new.source("archive")
                .unwrap()
                .unwrap()
                .kind()
                .unwrap()
                .as_str()
                .unwrap(),
            KIND_30
        );
        assert_eq!(
            new.source("archive")
                .unwrap()
                .unwrap()
                .value()
                .get("x-owner")
                .unwrap()
                .text(),
            src.additional_fields["x-owner"].text()
        );
        generated.document().as_document().value().text().to_owned()
    };
    for text in [&manual, &saved] {
        let document = Document::parse(text).unwrap();
        let draft = &document;
        let client = dynamic_openapi_client_reqwest::Client::new().unwrap();
        let provider = OpenApi::new(resources()).unwrap().with_transport(
            move |request: PreparedRequest, cancel: Cancellation| {
                let client = client.clone();
                async move {
                    dynamic_openapi_client_reqwest::advanced::execute(&client, &request, cancel)
                        .await
                }
            },
        );
        let invoker = operation::Invoker::new(
            document
                .validate()
                .unwrap()
                .conformant()
                .expect("conformant invocation document"),
            provider,
        )
        .with_evaluator(
            Arc::new(openbindings_json_schema_evaluator::DefaultEvaluator::new()),
            openbindings::ResourceSet::default(),
        );
        for row in &exchanges {
            let method = row["method"].as_str().unwrap().to_ascii_lowercase();
            let name = &draft
                .bindings()
                .unwrap()
                .unwrap()
                .into_iter()
                .find(|b| b.content().unwrap().get("method").unwrap().as_str() == Some(&method))
                .unwrap()
                .operation()
                .unwrap()
                .to_owned();
            let mut job = invoker.invoke(operation::Request::operation(name));
            let _ = job.write(json(row["input"].as_str().unwrap())).await;
            job.close();
            if let Some(error) = row.get("error") {
                let error_actual = job.next().await.unwrap().unwrap_err();
                assert_eq!(error_actual.code(), error.as_str().unwrap());
                assert!(error_actual.data().is_none());
                assert!(job.next().await.is_none());
            } else {
                if let Some(expected) = row["output"].as_str() {
                    assert_eq!(job.next().await.unwrap().unwrap().text(), expected);
                }
                assert!(job.next().await.is_none());
                assert!(job.outcome().unwrap().is_ok());
            }
        }
    }
    wire.join().unwrap();
    let draft = Document::parse(saved).unwrap();
    let src = &source_support::source(draft.source("archive").unwrap().unwrap());
    let count = Rc::new(Cell::new(0));
    let missing = OpenApi::default().with_transport(transport(
        count.clone(),
        Default::default(),
        200,
        "null",
        BodyState::Complete,
    ));
    assert!(!synth(&missing, src).await.unwrap().coverage().exhaustive());
    assert!(
        invoke(
            &missing,
            src,
            "/records/{key}",
            "get",
            &[r#"{"path":{"key":"a-7"}}"#],
            None
        )
        .await
        .is_err()
    );
    assert_eq!(count.get(), 0);
}
#[tokio::test]
async fn contexts_statuses_and_finite_boundaries_are_scoped_to_each_kind() {
    use openbindings_openapi::{SERVER_POINT_30, SERVER_POINT_31};
    for (kind, version, point, wrong) in [
        (KIND_30, "3.0.4", SERVER_POINT_30, SERVER_POINT_31),
        (KIND_31, "3.1.2", SERVER_POINT_31, SERVER_POINT_30),
    ] {
        let src=source(kind,&SIMPLE.replace("3.0.4",version).replace(r#"{"url":"https://version.example.invalid"}"#,r#"{"url":"https://version.example.invalid"},{"url":"https://second.example.invalid"}"#));
        let count = Rc::new(Cell::new(0));
        let targets = Rc::new(RefCell::new(vec![]));
        let provider = OpenApi::default().with_transport(transport(
            count.clone(),
            targets.clone(),
            404,
            "not json",
            BodyState::Complete,
        ));
        for (context, code) in [
            (json("{}"), "CONTEXT_REQUIRED"),
            (
                json(
                    &serde_json_compat::json!({"configuration":{wrong:{"url":"https://second.example.invalid"}}})
                        .to_string(),
                ),
                "CONTEXT_REQUIRED",
            ),
            (
                json(
                    &serde_json_compat::json!({"configuration":{point:{"url":"https://unlisted.example.invalid"}}})
                        .to_string(),
                ),
                "ERR_REFUSED",
            ),
        ] {
            let error = invoke(&provider, &src, "/ping", "get", &["{}"], Some(&context))
                .await
                .unwrap_err();
            assert_eq!(error.code(), code);
            if code == "CONTEXT_REQUIRED" {
                let challenge = error.data().unwrap().text();
                assert!(challenge.contains(kind));
                assert!(challenge.contains(point));
            } else {
                assert!(error.context_required().is_none());
                assert!(error.data().is_none());
            }
            assert_eq!(count.get(), 0);
        }
        let context = json(
            &serde_json_compat::json!({"configuration":{point:{"url":"https://second.example.invalid"}}})
                .to_string(),
        );
        let error = invoke(&provider, &src, "/ping", "get", &["{}"], Some(&context))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_EXECUTION_FAILED");
        assert!(error.data().is_none());
        assert_eq!(&*targets.borrow(), &["https://second.example.invalid/ping"]);
        assert_eq!(
            invoke(&provider, &src, "/ping", "get", &["{}"], None)
                .await
                .unwrap_err()
                .code(),
            "CONTEXT_REQUIRED"
        );
        assert_eq!(count.get(), 1);
        let src = source(kind, &SIMPLE.replace("3.0.4", version));
        for inputs in [vec!["{}", "{}"], vec![r#"{"body":null,"body":null}"#]] {
            assert_eq!(
                invoke(&provider, &src, "/ping", "get", &inputs, None)
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_REFUSED"
            );
            assert_eq!(count.get(), 1);
        }
        // No value makes the one request too; this host answers it with a 404.
        assert_eq!(
            invoke(&provider, &src, "/ping", "get", &[], None)
                .await
                .unwrap_err()
                .code(),
            "ERR_EXECUTION_FAILED"
        );
        assert_eq!(count.get(), 2);
        let bytes = src.content.as_ref().unwrap().bytes().len();
        for (limit, ok) in [(bytes, true), (bytes - 1, false)] {
            let mut options = Options::default();
            options.source_bytes = limit;
            assert_eq!(
                Inspector::new(OpenApi::new(options).unwrap())
                    .inspect(&src)
                    .await
                    .is_ok(),
                ok
            );
        }
        let mut options = Options::default();
        options.result_bytes = 1;
        assert!(synth(OpenApi::new(options).unwrap(), &src).await.is_err());
        let mut api: serde_json_compat::Value =
            serde_json_compat::from_str(&SIMPLE.replace("3.0.4", version)).unwrap();
        let mut op = api["paths"]["/ping"]["get"].clone();
        op["requestBody"] =
            serde_json_compat::json!({"required":true,"content":{"application/json":{}}});
        api["paths"]["/ping"] = serde_json_compat::json!({"post":op});
        let src = source(kind, &api.to_string());
        for (limit, ok) in [(4, true), (3, false)] {
            let mut options = Options::default();
            options.limits.body_bytes = limit;
            let calls = Rc::new(Cell::new(0));
            let p = OpenApi::new(options).unwrap().with_transport(transport(
                calls.clone(),
                Default::default(),
                204,
                "",
                BodyState::Complete,
            ));
            assert_eq!(
                invoke(&p, &src, "/ping", "post", &[r#"{"body":null}"#], None)
                    .await
                    .is_ok(),
                ok
            );
            assert_eq!(calls.get(), usize::from(ok));
            let before = calls.get();
            assert!(
                invoke(&p, &src, "/ping", "post", &["{}"], None)
                    .await
                    .is_err()
            );
            assert_eq!(calls.get(), before);
        }
        for (body, state) in [("{", BodyState::Complete), ("null", BodyState::Truncated)] {
            let p = OpenApi::default().with_transport(transport(
                Default::default(),
                Default::default(),
                200,
                body,
                state,
            ));
            assert_eq!(
                invoke(&p, &src, "/ping", "post", &[r#"{"body":null}"#], None)
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_EXECUTION_FAILED"
            );
        }
    }
}

#[tokio::test]
async fn get_request_body_uses_the_source_edition_semantics() {
    for (kind, version, has_body) in [(KIND_30, "3.0.4", false), (KIND_31, "3.1.2", true)] {
        let mut api: serde_json_compat::Value = serde_json_compat::from_str(SIMPLE).unwrap();
        api["openapi"] = version.into();
        api["paths"]["/ping"]["get"]["requestBody"] =
            serde_json_compat::json!({"required":true,"content":{"application/json":{}}});
        let src = source(kind, &api.to_string());
        let count = Rc::new(Cell::new(0));
        let sent = count.clone();
        let provider = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancel: Cancellation| {
                let sent = sent.clone();
                async move {
                    request
                        .invoke(
                            cancel,
                            &HostCapabilities::programmable(),
                            move |wire, _| async move {
                                // Both invoke paths below check literal body bytes via the prepared wire request.
                                sent.set(sent.get() + 1);
                                assert_eq!(
                                    wire.body().map(AsRef::as_ref),
                                    if has_body {
                                        Some(b"null".as_slice())
                                    } else {
                                        None
                                    }
                                );
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
                }
            },
        );
        let result = synth(&provider, &src).await.unwrap();
        assert!(result.coverage().strict_eligible());
        let draft = result.document().as_document();
        let input = draft
            .operations()
            .unwrap()
            .remove(0)
            .input()
            .unwrap()
            .to_owned();
        assert_eq!(
            input.get("properties").unwrap().get("body").is_some(),
            has_body
        );
        let accepted = if has_body { r#"{"body":null}"# } else { "{}" };
        let rejected = if has_body { "{}" } else { r#"{"body":null}"# };
        assert!(
            invoke(&provider, &src, "/ping", "get", &[accepted], None)
                .await
                .is_ok()
        );
        assert_eq!(
            invoke(&provider, &src, "/ping", "get", &[rejected], None)
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
        assert_eq!(count.get(), 1);
    }
}
