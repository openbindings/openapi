//! Public consumer with independently authored documents and literal exchanges.
use crate::{call, invoker, source};
use dynamic_openapi_client::{
    BodyState, Cancellation, DeliveredBody, Header, HeaderDelivery, HostCapabilities,
    PreparedRequest, Provenance, ResponseInput, TransportResult, UploadState,
};
use openbindings::{Document, JsonValue, ParsedDocument};
use openbindings_interface_synthesis::{
    CoverageStatus, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_invocation::{BindingRequest, BindingValue, binding, operation};
use openbindings_openapi::{EXPERIMENTAL_KIND, EXPERIMENTAL_KIND_30, OpenApi};
use openbindings_source_inspection::Inspector;
use std::{cell::Cell, rc::Rc, sync::Arc};

pub async fn suite(v30: bool) {
    let (kind, description, manual) = if v30 {
        (
            EXPERIMENTAL_KIND_30,
            include_str!("crud-fixtures/service-3.0.4.json"),
            include_str!("crud-fixtures/manual-3.0.4.obi.json"),
        )
    } else {
        (
            EXPERIMENTAL_KIND,
            include_str!("crud-fixtures/service-3.1.2.json"),
            include_str!("crud-fixtures/manual-3.1.2.obi.json"),
        )
    };
    let src = source(kind, description);
    let provider = OpenApi::default();
    let inventory = Inspector::new(&provider).inspect(&src).await.unwrap();
    assert_eq!(inventory.targets.len(), 6);
    assert!(inventory.targets.iter().all(|t| t.operation.is_some()));
    let sources = [SynthesisSource {
        name: Some("recordsApi"),
        source: &src,
    }];
    let synthesis = Synthesizer::new(&provider)
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &sources,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(synthesis.coverage().strict_eligible());
    assert!(
        synthesis
            .coverage()
            .entries()
            .iter()
            .any(|e| e.status == CoverageStatus::Lossy)
    );
    let saved = synthesis.document().parsed().value().text().to_owned();
    drop(synthesis);
    drop(provider);
    let exchanges: serde_json::Value =
        serde_json::from_str(include_str!("crud-fixtures/exchanges.json")).unwrap();
    for saved in [manual, &saved] {
        let parsed = ParsedDocument::parse(saved).unwrap();
        let draft = Document::from_json(parsed.value()).unwrap();
        assert_eq!(
            draft.sources.as_ref().unwrap()["recordsApi"]
                .content
                .as_ref()
                .unwrap()
                .get("document")
                .and_then(|v| v.as_str()),
            Some(description)
        );
        assert_eq!(draft.sources.as_ref().unwrap()["recordsApi"].kind, kind);
        // Both authoring routes start with a fresh provider and source cache.
        for exchange in exchanges.as_array().unwrap() {
            let text = |field| exchange[field].as_str().unwrap();
            let path = text("path");
            let method = text("method").to_ascii_lowercase();
            let binding = draft
                .bindings
                .as_ref()
                .unwrap()
                .values()
                .find(|b| {
                    let content = b.content.as_ref().unwrap();
                    content.get("path").and_then(|v| v.as_str()) == Some(path)
                        && content.get("method").and_then(|v| v.as_str()) == Some(&method)
                })
                .unwrap();
            let context = JsonValue::parse(serde_json::json!({"credentials":{"Token":"crud-token"},"configuration":{format!("{kind}.security"):{"origins":{"Token":["https://crud.example.test"]}}}}).to_string()).unwrap();
            for operation_path in [false, true] {
                let calls = Rc::new(Cell::new(0));
                let recorded = calls.clone();
                let expected = exchange.clone();
                let provider = OpenApi::default().with_transport(
                    move |request: PreparedRequest, cancel: Cancellation| {
                        let recorded = recorded.clone();
                        let expected = expected.clone();
                        async move {
                            #[cfg(all(target_arch = "wasm32", feature = "wasm-probe"))]
                            wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(
                                &wasm_bindgen::JsValue::NULL,
                            ))
                            .await
                            .unwrap();
                            request
                                .invoke(
                                    cancel,
                                    &HostCapabilities::programmable(),
                                    move |wire, _| async move {
                                        recorded.set(recorded.get() + 1);
                                        assert_eq!(
                                            wire.method.as_str(),
                                            expected["method"].as_str().unwrap()
                                        );
                                        assert_eq!(
                                            wire.target,
                                            expected["target"].as_str().unwrap()
                                        );
                                        assert_eq!(
                                            wire.body.as_deref(),
                                            expected["request"].as_str().map(str::as_bytes)
                                        );
                                        let auth: Vec<_> = wire
                                            .headers
                                            .iter()
                                            .filter(|h| {
                                                h.name().eq_ignore_ascii_case("authorization")
                                            })
                                            .collect();
                                        if expected["method"] == "PATCH" {
                                            assert_eq!(auth.len(), 1);
                                            assert_eq!(auth[0].value(), b"Bearer crud-token");
                                        } else {
                                            assert!(auth.is_empty());
                                        }
                                        let body = expected["response"].as_str().unwrap();
                                        let mut response = ResponseInput::new(
                                            expected["status"].as_u64().unwrap() as u16,
                                            if body.is_empty() {
                                                vec![]
                                            } else {
                                                vec![Header::new(
                                                    "content-type",
                                                    "application/json",
                                                )]
                                            },
                                            DeliveredBody {
                                                bytes: Arc::from(body.as_bytes()),
                                                state: BodyState::Complete,
                                                provenance: Provenance::Wire,
                                            },
                                        );
                                        response.header_delivery = HeaderDelivery::Complete;
                                        TransportResult {
                                            response: Some(response),
                                            upload: UploadState::Complete,
                                            error_detail: None,
                                        }
                                    },
                                )
                                .await
                        }
                    },
                );
                let output = if operation_path {
                    call(
                        &invoker(saved, provider),
                        operation::Request::operation(&binding.operation).with_context(&context),
                        text("input"),
                    )
                    .await
                    .unwrap()
                } else {
                    let invoker = binding::Invoker::new(provider);
                    let value = BindingValue::from(binding);
                    let mut job = invoker.invoke(BindingRequest {
                        source: &src,
                        binding: &value,
                        context: Some(&context),
                    });
                    let _ = job.write(JsonValue::parse(text("input")).unwrap()).await;
                    job.close();
                    let mut output = None;
                    while let Some(value) = job.next().await {
                        assert!(output.is_none());
                        output = Some(value.unwrap().text().to_owned());
                    }
                    job.outcome().unwrap().unwrap();
                    output
                };
                assert_eq!(output.as_deref(), exchange["output"].as_str());
                assert_eq!(calls.get(), 1);
            }
        }
    }
}
