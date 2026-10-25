//! The two private kinds' labels and configuration points, and the retired labels,
//! which every job refuses exactly as it refuses any other unknown kind.
use dynamic_openapi_client::{
    Cancellation, Outcome,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::{JsonValue, Source};
use openbindings_interface_synthesis::{
    InterfaceSynthesizer, SynthesisErrorKind, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_invocation::{BindingRequest, BindingValue, binding, binding::BindingInvoker};
use openbindings_openapi::{
    KIND_30, KIND_31, OpenApi, SECURITY_POINT_30, SECURITY_POINT_31, SERVER_POINT_30,
    SERVER_POINT_31,
};
use openbindings_source_inspection::{
    CancellationToken, InspectionErrorKind, Inspector, SourceInspector,
};
use serde_json_compat::json;
use std::{cell::Cell, rc::Rc, sync::Arc};

/// The earlier private labels, which no job supports any longer.
const RETIRED: [&str; 2] = [
    "local.openapi-json-probe.r1",
    "local.openapi30-json-probe.r1",
];

fn description(version: &str, servers: &[&str]) -> String {
    json!({
        "openapi": version,
        "info": {"title": "Kinds", "version": "1"},
        "servers": servers.iter().map(|url| json!({"url": url})).collect::<Vec<_>>(),
        "paths": {"/ping": {"get": {"responses": {"204": {"description": "empty"}}}}}
    })
    .to_string()
}
fn source(kind: &str, content: serde_json_compat::Value) -> Source {
    Source {
        kind: kind.into(),
        content: Some(JsonValue::parse(content.to_string()).unwrap()),
        ..Default::default()
    }
}
fn counting_host(
    calls: Rc<Cell<usize>>,
) -> impl Fn(PreparedRequest, Cancellation) -> std::pin::Pin<Box<dyn Future<Output = Outcome>>> {
    move |request, cancellation| {
        let calls = calls.clone();
        Box::pin(async move {
            request
                .invoke(
                    cancellation,
                    &HostCapabilities::programmable(),
                    move |_, _| async move {
                        calls.set(calls.get() + 1);
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
async fn invoke<P: BindingInvoker>(
    provider: P,
    source: &Source,
    context: Option<&JsonValue>,
) -> Result<(), openbindings_invocation::InvocationError> {
    let binding = BindingValue {
        content: Some(JsonValue::parse(r#"{"path":"/ping","method":"get"}"#).unwrap()),
        ..Default::default()
    };
    let invoker = binding::Invoker::new(provider);
    let mut job = invoker.invoke(BindingRequest {
        source,
        binding: &binding,
        context,
    });
    let _ = job.write(JsonValue::parse("{}").unwrap()).await;
    job.close();
    while let Some(value) = job.next().await {
        value?;
    }
    job.outcome().expect("terminal")
}

#[test]
fn labels_and_configuration_points_are_qualified_by_the_kind() {
    assert_eq!(KIND_31, "local.openapi-3.1.r2");
    assert_eq!(KIND_30, "local.openapi-3.0.r2");
    assert_eq!(SERVER_POINT_31, format!("{KIND_31}.server"));
    assert_eq!(SECURITY_POINT_31, format!("{KIND_31}.security"));
    assert_eq!(SERVER_POINT_30, format!("{KIND_30}.server"));
    assert_eq!(SECURITY_POINT_30, format!("{KIND_30}.security"));
    let provider = OpenApi::default();
    // Invocation needs a host; inspection and synthesis do not.
    let invoker = OpenApi::default().with_transport(counting_host(Rc::default()));
    let advertised = |kinds: Vec<String>| assert_eq!(kinds, [KIND_31, KIND_30]);
    advertised(
        SourceInspector::supported_kinds(&provider)
            .into_iter()
            .map(|k| k.kind)
            .collect(),
    );
    advertised(
        InterfaceSynthesizer::supported_kinds(&provider)
            .into_iter()
            .map(|k| k.kind)
            .collect(),
    );
    advertised(
        BindingInvoker::supported_kinds(&invoker)
            .into_iter()
            .map(|k| k.kind)
            .collect(),
    );
    for kind in [KIND_31, KIND_30] {
        assert!(SourceInspector::supports_kind(&provider, kind));
        assert!(InterfaceSynthesizer::supports_kind(&provider, kind));
        assert!(BindingInvoker::supports_kind(&invoker, kind));
    }
    for kind in RETIRED {
        assert!(!SourceInspector::supports_kind(&provider, kind));
        assert!(!InterfaceSynthesizer::supports_kind(&provider, kind));
        assert!(!BindingInvoker::supports_kind(&invoker, kind));
    }
}

#[tokio::test]
async fn retired_labels_are_refused_as_unknown_kinds_before_any_work() {
    for (version, current) in [("3.1.2", KIND_31), ("3.0.4", KIND_30)] {
        let text = description(version, &["https://kinds.example.test"]);
        // Both the earlier carrier and the current one; neither is read under a retired label.
        let carriers = [
            json!({"document": text, "retrievalUri": "https://descriptions.example.invalid/api.json"}),
            json!({"document": text, "location": "https://descriptions.example.invalid/api.json"}),
            json!("not even a content object"),
        ];
        let provider = OpenApi::default();
        let calls = Rc::new(Cell::new(0));
        let invoker = OpenApi::default().with_transport(counting_host(calls.clone()));
        // The same content under a current label works, and warms both caches.
        let warm = source(current, carriers[1].clone());
        Inspector::new(&provider).inspect(&warm).await.unwrap();
        invoke(&invoker, &warm, None).await.unwrap();
        assert_eq!(calls.get(), 1);
        for kind in RETIRED.into_iter().chain(["unknown.kind"]) {
            for content in &carriers {
                let src = source(kind, content.clone());
                let inspection = Inspector::new(&provider).inspect(&src).await.unwrap_err();
                assert_eq!(inspection.kind(), InspectionErrorKind::UnsupportedKind);
                assert_eq!(
                    inspection.code(),
                    "openbindings.inspection.unsupported-kind"
                );
                // Called directly, the provider refuses the kind before reading content.
                let direct = SourceInspector::inspect(&provider, &src, CancellationToken::new())
                    .await
                    .unwrap_err();
                assert_eq!(direct.kind(), InspectionErrorKind::UnsupportedKind);
                assert_eq!(direct.code(), "openbindings.openapi.unsupported-kind");
                let sources = [SynthesisSource {
                    name: None,
                    source: &src,
                }];
                let request = SynthesisRequest {
                    sources: &sources,
                    ..Default::default()
                };
                let synthesis = Synthesizer::new(&provider)
                    .synthesize(&request)
                    .await
                    .unwrap_err();
                assert_eq!(synthesis.kind(), SynthesisErrorKind::UnsupportedKind);
                assert_eq!(synthesis.code(), "openbindings.synthesis.unsupported-kind");
                let direct =
                    InterfaceSynthesizer::synthesize(&provider, &request, CancellationToken::new())
                        .await
                        .unwrap_err();
                assert_eq!(direct.kind(), SynthesisErrorKind::UnsupportedKind);
                assert_eq!(direct.code(), "openbindings.openapi.unsupported-kind");
                let invocation = invoke(&invoker, &src, None).await.unwrap_err();
                assert_eq!(invocation.code(), "ERR_REFUSED");
                assert!(invocation.data().is_none());
            }
        }
        assert_eq!(calls.get(), 1, "a refused kind never reaches the host");
    }
}

#[tokio::test]
async fn retired_configuration_points_select_nothing() {
    for (version, kind, server_point) in [
        ("3.1.2", KIND_31, SERVER_POINT_31),
        ("3.0.4", KIND_30, SERVER_POINT_30),
    ] {
        let text = description(
            version,
            &["https://one.example.test", "https://two.example.test"],
        );
        let src = source(kind, json!({"document": text}));
        let calls = Rc::new(Cell::new(0));
        let provider = OpenApi::default().with_transport(counting_host(calls.clone()));
        let retired = JsonValue::parse(
            json!({"configuration": {
                "local.openapi-json-probe.r1.server": "https://two.example.test",
                "local.openapi30-json-probe.r1.server": "https://two.example.test"
            }})
            .to_string(),
        )
        .unwrap();
        let error = invoke(&provider, &src, Some(&retired)).await.unwrap_err();
        assert_eq!(error.code(), "CONTEXT_REQUIRED");
        let challenge = error.data().unwrap().text().to_owned();
        assert!(challenge.contains(server_point));
        assert!(!challenge.contains("json-probe"));
        assert_eq!(calls.get(), 0);
        let current = JsonValue::parse(
            json!({"configuration": {server_point: {"url": "https://two.example.test"}}})
                .to_string(),
        )
        .unwrap();
        invoke(&provider, &src, Some(&current)).await.unwrap();
        assert_eq!(calls.get(), 1);
    }
}
