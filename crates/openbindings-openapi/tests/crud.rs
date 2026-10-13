//! Finite success correspondence, independently specified at the public role seam.
use dynamic_openapi_client::{
    BodyState, Cancellation, DeliveredBody, Header, HeaderDelivery, HostCapabilities,
    PreparedRequest, Provenance, ResponseInput, TransportResult, UploadState,
};
use openbindings::{JsonValue, Source, WorkControl};
use openbindings_interface_synthesis::{
    CoverageStatus, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_invocation::{BindingRequest, BindingValue, binding};
use openbindings_openapi::{EXPERIMENTAL_KIND, EXPERIMENTAL_KIND_30, OpenApi};
use serde_json_compat::{Value, json};
use std::{cell::Cell, rc::Rc, sync::Arc};

fn source(v30: bool, method: &str, responses: Value, body: bool) -> Source {
    let mut op = json!({"responses":responses});
    if body {
        op["requestBody"] = json!({"required":true,"content":{"application/json":{"schema":{}}}});
    }
    let doc = json!({"openapi":if v30 {"3.0.4"} else {"3.1.2"},"info":{"title":"CRUD boundaries","version":"1"},"servers":[{"url":"https://crud.example.test"}],"paths":{"/records":{method:op}},"components":{"responses":{"Empty":{"description":"no representation"}}}});
    Source {
        kind: if v30 {
            EXPERIMENTAL_KIND_30
        } else {
            EXPERIMENTAL_KIND
        }
        .into(),
        content: Some(JsonValue::parse(json!({"document":doc.to_string()}).to_string()).unwrap()),
        ..Default::default()
    }
}
fn json_response() -> Value {
    json!({"description":"JSON","content":{"application/json":{}}})
}
fn empty_response() -> Value {
    json!({"description":"no representation"})
}
struct Delivery {
    status: u16,
    body: &'static str,
    state: BodyState,
    headers: HeaderDelivery,
}
async fn invoke(
    src: &Source,
    method: &str,
    input: &str,
    delivery: Delivery,
    cancelled: bool,
) -> (Result<Vec<String>, String>, usize) {
    let count = Rc::new(Cell::new(0));
    let recorded = count.clone();
    let expected_body = JsonValue::parse(input)
        .unwrap()
        .get("body")
        .map(|v| v.text().as_bytes().to_vec());
    let provider =
        OpenApi::default().with_transport(move |request: PreparedRequest, cancel: Cancellation| {
            let recorded = recorded.clone();
            let expected_body = expected_body.clone();
            async move {
                request
                    .invoke(
                        cancel,
                        &HostCapabilities::programmable(),
                        move |wire, _| async move {
                            recorded.set(recorded.get() + 1);
                            assert_eq!(wire.body.as_deref(), expected_body.as_deref());
                            let mut response = ResponseInput::new(
                                delivery.status,
                                vec![Header::new("content-type", "application/json")],
                                DeliveredBody {
                                    bytes: Arc::from(delivery.body.as_bytes()),
                                    state: delivery.state,
                                    provenance: Provenance::Wire,
                                },
                            );
                            response.header_delivery = delivery.headers;
                            TransportResult {
                                response: Some(response),
                                upload: UploadState::Complete,
                                error_detail: None,
                            }
                        },
                    )
                    .await
            }
        });
    let invoker = binding::Invoker::new(provider);
    let binding = BindingValue {
        content: Some(
            JsonValue::parse(json!({"path":"/records","method":method}).to_string()).unwrap(),
        ),
        ..Default::default()
    };
    let control = WorkControl::new();
    if cancelled {
        control.cancel();
    }
    let mut call = invoker.invoke_with_control(
        BindingRequest {
            source: src,
            binding: &binding,
            context: None,
        },
        &control,
    );
    let _ = call.write(JsonValue::parse(input).unwrap()).await;
    call.close();
    let mut output = Vec::new();
    while let Some(value) = call.next().await {
        match value {
            Ok(v) => output.push(v.text().to_owned()),
            Err(e) => {
                assert!(output.is_empty());
                return (Err(e.code().into()), count.get());
            }
        }
    }
    (
        call.outcome()
            .unwrap()
            .map(|()| output)
            .map_err(|e| e.code().into()),
        count.get(),
    )
}
async fn run(
    src: &Source,
    method: &str,
    input: &str,
    status: u16,
    body: &'static str,
) -> (Result<Vec<String>, String>, usize) {
    invoke(
        src,
        method,
        input,
        Delivery {
            status,
            body,
            state: BodyState::Complete,
            headers: HeaderDelivery::Complete,
        },
        false,
    )
    .await
}

#[tokio::test]
async fn crud_methods_preserve_required_absent_and_null_bodies() {
    for v30 in [true, false] {
        for method in ["put", "patch"] {
            let src = source(v30, method, json!({"200":json_response()}), true);
            assert_eq!(
                run(&src, method, r#"{"body":null}"#, 200, "null").await,
                (Ok(vec!["null".into()]), 1)
            );
            assert_eq!(
                run(&src, method, "{}", 200, "null").await,
                (Err("ERR_REFUSED".into()), 0)
            );
        }
        let src = source(v30, "delete", json!({"202":empty_response()}), true);
        // The upstream editions deliberately differ on DELETE requestBody.
        for body in [false, true] {
            let result = run(
                &src,
                "delete",
                if body { r#"{"body":null}"# } else { "{}" },
                202,
                "",
            )
            .await;
            assert_eq!(
                result,
                if body != v30 {
                    (Ok(vec![]), 1)
                } else {
                    (Err("ERR_REFUSED".into()), 0)
                }
            );
        }
    }
}

#[tokio::test]
async fn successful_response_correspondence_and_precedence() {
    for v30 in [true, false] {
        for (responses, status, bytes, expected) in [
            (json!({"202":empty_response()}), 202, "", Some(vec![])),
            (json!({"202":empty_response()}), 202, "null", None),
            (json!({"200":json_response()}), 200, "", None),
            (
                json!({"200":json_response()}),
                200,
                "null",
                Some(vec!["null".into()]),
            ),
            (json!({"200":json_response()}), 200, "{", None),
            (
                json!({"201":empty_response(),"2XX":json_response(),"default":empty_response()}),
                201,
                "",
                Some(vec![]),
            ),
            (
                json!({"201":empty_response(),"2XX":json_response(),"default":empty_response()}),
                202,
                "",
                None,
            ),
            (
                json!({"200":json_response(),"2XX":empty_response()}),
                200,
                "null",
                Some(vec!["null".into()]),
            ),
            (
                json!({"200":json_response(),"2XX":empty_response()}),
                202,
                "",
                Some(vec![]),
            ),
            (json!({"default":empty_response()}), 202, "", Some(vec![])),
            (json!({"default":empty_response()}), 202, "null", None),
            (
                json!({"202":{"$ref":"#/components/responses/Empty"}}),
                202,
                "",
                Some(vec![]),
            ),
            (
                json!({"200":json_response()}),
                201,
                "null",
                Some(vec!["null".into()]),
            ),
            (json!({"200":json_response()}), 201, "", None),
            (json!({"204":empty_response()}), 204, "", Some(vec![])),
            (json!({"204":empty_response()}), 204, "null", None),
            (json!({"205":empty_response()}), 205, "", Some(vec![])),
            (json!({"205":empty_response()}), 205, "null", None),
            (json!({"2XX":json_response()}), 205, "null", None),
        ] {
            let src = source(v30, "put", responses, false);
            let (result, calls) = run(&src, "put", "{}", status, bytes).await;
            assert_eq!(calls, 1);
            assert_eq!(
                result,
                expected.ok_or_else(|| "ERR_EXECUTION_FAILED".into()),
                "v30={v30}, status={status}, bytes={bytes}"
            );
        }
        let src = source(v30, "delete", json!({"default":empty_response()}), false);
        assert_eq!(
            run(&src, "delete", "{}", 409, "").await,
            (Err(format!("{}.http-status", src.kind)), 1)
        );
    }
}

#[tokio::test]
async fn empty_delivery_still_requires_complete_evidence_and_obeys_cancellation() {
    for v30 in [true, false] {
        let src = source(v30, "delete", json!({"202":empty_response()}), false);
        for (state, headers, cancelled, error, calls) in [
            (
                BodyState::Truncated,
                HeaderDelivery::Complete,
                false,
                "ERR_EXECUTION_FAILED",
                1,
            ),
            (
                BodyState::Complete,
                HeaderDelivery::Unspecified,
                false,
                "ERR_EXECUTION_FAILED",
                1,
            ),
            (
                BodyState::Complete,
                HeaderDelivery::Complete,
                true,
                "ERR_CANCELLED",
                0,
            ),
        ] {
            assert_eq!(
                invoke(
                    &src,
                    "delete",
                    "{}",
                    Delivery {
                        status: 202,
                        body: "",
                        state,
                        headers
                    },
                    cancelled
                )
                .await,
                (Err(error.into()), calls)
            );
        }
    }
}

#[tokio::test]
async fn unsupported_methods_and_media_remain_explicit() {
    for v30 in [true, false] {
        for method in ["head", "options", "trace"] {
            let src = source(v30, method, json!({"202":empty_response()}), false);
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
            assert_eq!(
                result.coverage().entries()[0].status,
                CoverageStatus::ImplementationUnsupported
            );
            assert_eq!(
                run(&src, method, "{}", 202, "").await,
                (Err("ERR_REFUSED".into()), 0)
            );
        }
        let src = source(
            v30,
            "patch",
            json!({"200":{"description":"text","content":{"text/plain":{}}}}),
            true,
        );
        assert_eq!(
            run(&src, "patch", r#"{"body":{}}"#, 200, "{}").await,
            (Err("ERR_REFUSED".into()), 0)
        );
    }
}

#[tokio::test]
async fn usable_crud_targets_survive_unsupported_neighbors_and_analysis_limits_hold() {
    for v30 in [true, false] {
        let mut src = source(v30, "put", json!({"200":json_response()}), false);
        let mut document: Value = serde_json_compat::from_str(
            src.content
                .as_ref()
                .unwrap()
                .get("document")
                .unwrap()
                .as_str()
                .unwrap(),
        )
        .unwrap();
        document["paths"]["/unimplemented"] =
            json!({"head":{"responses":{"200":empty_response()}}});
        src.content =
            Some(JsonValue::parse(json!({"document":document.to_string()}).to_string()).unwrap());
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
        assert!(!result.coverage().strict_eligible());
        assert_eq!(
            result
                .coverage()
                .entries()
                .iter()
                .filter(|e| e.status == CoverageStatus::Represented)
                .count(),
            1
        );
        assert_eq!(
            result
                .coverage()
                .entries()
                .iter()
                .filter(|e| e.status == CoverageStatus::ImplementationUnsupported)
                .count(),
            1
        );
        assert_eq!(
            run(&src, "put", "{}", 200, "null").await,
            (Ok(vec!["null".into()]), 1)
        );
        let mut options = openbindings_openapi::Options::default();
        options.result_bytes = 1;
        assert!(
            openbindings_source_inspection::Inspector::new(OpenApi::new(options).unwrap())
                .inspect(&src)
                .await
                .is_err()
        );
    }
}
