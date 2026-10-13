//! External consumer: inspection, synthesis, save/reload and finite invocation.
use dynamic_openapi_client::{
    BodyState, DeliveredBody, Header, HeaderDelivery, Provenance, ResponseInput, TransportResult,
    UploadState,
};
use openbindings::{JsonValue, ParsedDocument, Source};
use openbindings_interface_synthesis::{SynthesisRequest, SynthesisSource, Synthesizer};
use openbindings_invocation::{InvocationError, binding::BindingInvoker, operation};
use openbindings_openapi::OpenApi;
use std::sync::Arc;
pub fn source(kind: &str, text: &str) -> Source {
    Source {
        kind: kind.into(),
        content: Some(JsonValue::parse(serde_json::json!({"document":text}).to_string()).unwrap()),
        ..Default::default()
    }
}
pub const DESCRIPTION: &str = r#"{"openapi":"3.1.2","info":{"title":"Consumer","version":"1"},"servers":[{"url":"https://consumer.invalid"}],"paths":{"/items":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":true}}},"responses":{"200":{"description":"echo","content":{"application/json":{"schema":true}}}}}}}}"#;
pub const DESCRIPTION_30: &str = r#"{"openapi":"3.0.4","info":{"title":"Consumer 3.0","version":"1"},"servers":[{"url":"https://consumer.invalid"}],"paths":{"/items":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","nullable":true}}}},"responses":{"200":{"description":"echo","content":{"application/json":{"schema":{"type":"object","nullable":true}}}}}}}}}"#;
pub const INPUT: &str = r#"{"body":{"integer":9007199254740993,"label":"雪"}}"#;
pub const OUTPUT: &str = r#"{"integer":9007199254740993,"label":"雪"}"#;
pub async fn author(kind: &str, text: &str) -> String {
    let source = source(kind, text);
    let provider = OpenApi::default();
    let inventory = openbindings_source_inspection::Inspector::new(&provider)
        .inspect(&source)
        .await
        .unwrap();
    assert_eq!(inventory.targets.len(), 1);
    let inputs = [SynthesisSource {
        name: Some("owned_api"),
        source: &source,
    }];
    let result = Synthesizer::new(provider)
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &inputs,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(result.coverage().strict_eligible());
    result.document().parsed().value().text().to_owned()
}
pub fn invoker<P: BindingInvoker>(saved: &str, provider: P) -> operation::Invoker<P> {
    operation::Invoker::new(ParsedDocument::parse(saved).unwrap(), provider)
        .with_evaluator(
            Arc::new(openbindings_json_schema_evaluator::DefaultEvaluator::new()),
            openbindings::ResourceSet::default(),
        )
        .unwrap()
}
/// Caller policy for this finite, at-most-one-output experiment. Preserve the
/// authoritative terminal after a local write error; retain output until success.
pub async fn call<P: BindingInvoker>(
    invoker: &operation::Invoker<P>,
    request: operation::Request<'_>,
    input: &str,
) -> Result<Option<String>, InvocationError> {
    let mut job = invoker.invoke(request);
    let _ = job
        .write(JsonValue::parse(input).map_err(|_| InvocationError::refused())?)
        .await;
    job.close();
    let mut output = None;
    while let Some(value) = job.next().await {
        let value = value?;
        if output.is_some() {
            return Err(InvocationError::execution_failed());
        }
        output = Some(value.text().to_owned());
    }
    job.outcome().expect("drained terminal")?;
    Ok(output)
}
pub fn reply(body: &str) -> TransportResult {
    let mut response = ResponseInput::new(
        200,
        vec![Header::new("content-type", "application/json")],
        DeliveredBody {
            bytes: Arc::from(body.as_bytes()),
            state: BodyState::Complete,
            provenance: Provenance::Wire,
        },
    );
    response.header_delivery = HeaderDelivery::Complete;
    TransportResult {
        upload: UploadState::Complete,
        response: Some(response),
        error_detail: None,
    }
}
pub fn owners() -> (usize, usize) {
    (
        openbindings_internal_json::backend::live_arenas(),
        dynamic_openapi_client::live_json_owners(),
    )
}
#[cfg(all(target_arch = "wasm32", feature = "wasm-probe"))]
mod wasm;

pub use serde_json as wire_json;
pub mod auth;

pub mod auth_acceptance;
pub mod crud;
