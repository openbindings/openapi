//! CPU comparison: no socket latency, no advertised HTTP-parity claim.
use dynamic_openapi_client as client;
use openapi_json_path_consumer::{reply, source};
use openbindings::{
    Binding, Conformant, Direction, Document, Edit, JsonValue, Operation, ResourceSet,
    ValueContractPreparation, ValueOutcome,
};
use openbindings_interface_synthesis::{SynthesisRequest, SynthesisSource, Synthesizer};
use openbindings_invocation::{BindingRequest, BindingValue, binding, operation};
use openbindings_openapi::OpenApi;
use std::{
    collections::BTreeMap,
    future::Future,
    hint::black_box,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
fn ready<F: Future>(future: F) -> F::Output {
    let mut f = std::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..1000 {
        if let Poll::Ready(value) = f.as_mut().poll(&mut cx) {
            return value;
        }
    }
    panic!("benchmark host unexpectedly pending")
}
fn sample(mut f: impl FnMut()) -> Vec<f64> {
    f();
    (0..7)
        .map(|_| {
            let start = Instant::now();
            let mut n = 0u64;
            while start.elapsed() < Duration::from_millis(50) {
                f();
                n += 1;
            }
            start.elapsed().as_nanos() as f64 / n as f64
        })
        .collect()
}
fn description(n: usize, method: &str, version: &str) -> String {
    let paths:BTreeMap<_,_>=(0..n).map(|i|(format!("/p{i}"),serde_json::json!({method:{"requestBody":if method=="post" {serde_json::json!({"content":{"application/json":{}}})}else{serde_json::Value::Null},"responses":{"200":{"description":"json","content":{"application/json":{}}}}}}))).collect();
    let mut v = serde_json::json!({"openapi":version,"info":{"title":"Measure","version":"1"},"servers":[{"url":"https://measure.invalid"}],"paths":paths});
    if method == "get" {
        for value in v["paths"].as_object_mut().unwrap().values_mut() {
            value["get"].as_object_mut().unwrap().remove("requestBody");
        }
    }
    v.to_string()
}
fn client_doc(text: &str) -> client::Document {
    client::Document::parse(text.as_bytes(), None, client::Limits::default()).unwrap()
}
fn schema() -> JsonValue {
    JsonValue::parse(r#"{"type":"object","properties":{"parameters":{"type":"object","properties":{"path":{"type":"object","properties":{},"required":[],"additionalProperties":false},"query":{"type":"object","properties":{},"required":[],"additionalProperties":false}},"required":[],"additionalProperties":false}},"required":[],"additionalProperties":false}"#).unwrap()
}
fn authored(source: &openbindings::Source, document: &client::Document) -> Conformant<Document> {
    let mut draft = Document::new();
    let sources = BTreeMap::from([("api".into(), source.clone())]);
    let mut operations = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    let schema = schema();
    for (i, op) in document.operations().enumerate() {
        let key = format!(
            "get_{}_{}",
            i + 1,
            op.path()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        );
        operations.insert(
            key.clone(),
            Operation {
                input: Some(schema.clone()),
                ..Default::default()
            },
        );
        bindings.insert(
            key.clone(),
            Binding {
                operation: key,
                source: "api".into(),
                content: Some(
                    JsonValue::parse(
                        serde_json::json!({"path":op.path(),"method":"get"}).to_string(),
                    )
                    .unwrap(),
                ),
                ..Default::default()
            },
        );
    }
    draft
        .edit(&[
            Edit::SetOperations { value: &operations },
            Edit::SetSources { value: &sources },
            Edit::SetBindings { value: &bindings },
        ])
        .unwrap();
    draft.validate().unwrap().conformant().unwrap()
}
struct Echo(JsonValue);
impl binding::BindingInvoker for Echo {
    type Invocation<'a> = std::pin::Pin<
        Box<dyn Future<Output = Result<(), openbindings_invocation::InvocationError>> + 'a>,
    >;
    fn supports_kind(&self, _: &str) -> bool {
        true
    }
    fn invoke<'a>(
        &'a self,
        _: BindingRequest<'a>,
        io: openbindings_invocation::ProviderIo,
        _: openbindings_source_inspection::CancellationToken,
    ) -> Self::Invocation<'a> {
        Box::pin(async move {
            let (mut input, mut output) = io.split();
            while input.next().await.is_some() {}
            output.emit(self.0.clone()).await
        })
    }
}
fn main() {
    let mode = std::env::args().nth(1).unwrap();
    let version = std::env::args().nth(2).unwrap_or_else(|| "3.1.2".into());
    let kind = match version.as_str() {
        "3.0.4" => openbindings_openapi::EXPERIMENTAL_KIND_30,
        "3.1.2" => openbindings_openapi::EXPERIMENTAL_KIND,
        _ => panic!("unsupported benchmark version"),
    };
    let candidate = mode == "candidate";
    let mut rows = Vec::new();
    for n in [1, 32, 512] {
        let text = description(n, "get", &version);
        let src = source(kind, &text);
        for stage in ["source-generation", "client-source-preparation"] {
            let samples = sample(|| {
                if stage == "source-generation" {
                    black_box(description(n, "get", &version));
                } else {
                    black_box(client_doc(&text));
                }
            });
            rows.push(
                serde_json::json!({"workload":"stage","stage":stage,"n":n,"samples_ns":samples}),
            );
        }
        let document = client_doc(&text);
        let provider = OpenApi::default();
        let inputs = [SynthesisSource {
            name: None,
            source: &src,
        }];
        let req = SynthesisRequest {
            sources: &inputs,
            ..Default::default()
        };
        let facade = Synthesizer::new(&provider);
        let initial = ready(facade.synthesize_with_coverage(&req)).unwrap();
        assert_eq!(
            initial
                .document()
                .as_document()
                .clone()
                .operations()
                .unwrap()
                .len(),
            n
        );
        // Baseline and candidate contain the same authored source and operation schemas.
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                authored(&src, &document).as_document().value().text()
            )
            .unwrap(),
            serde_json::from_str::<serde_json::Value>(
                initial.document().as_document().value().text()
            )
            .unwrap()
        );
        for cold in [false, true] {
            let samples = sample(|| {
                if candidate {
                    if cold {
                        let p = OpenApi::default();
                        black_box(
                            ready(Synthesizer::new(p).synthesize_with_coverage(&req)).unwrap(),
                        );
                    } else {
                        black_box(ready(facade.synthesize_with_coverage(&req)).unwrap());
                    }
                } else if cold {
                    black_box(authored(&src, &client_doc(&text)));
                } else {
                    black_box(authored(&src, &document));
                }
            });
            rows.push(
                serde_json::json!({"workload":"synthesis","n":n,"cold":cold,"samples_ns":samples}),
            );
        }
    }
    for method in ["get", "post"] {
        for bytes in [1024, 65536] {
            let text = description(1, method, &version);
            let src = source(kind, &text);
            let document = client_doc(&text);
            let op = document.operations().next().unwrap();
            let body = format!("\"{}\"", "x".repeat(bytes - 2));
            let envelope = if method == "post" {
                format!("{{\"body\":{body}}}")
            } else {
                "{}".into()
            };
            let value = JsonValue::parse(&envelope).unwrap();
            let input_body = (method == "post")
                .then(|| client::exact::Value::parse(&body, client::Limits::default()).unwrap());
            let prepare = |operation: &client::Operation| {
                let mut request = operation.request();
                if let Some(body) = &input_body {
                    request = request.json_exact(body);
                }
                request.prepare().unwrap()
            };
            let response = body.clone();
            let host = move |request: client::transport::PreparedRequest,
                             cancel: client::Cancellation| {
                let response = response.clone();
                async move {
                    request
                        .invoke(
                            cancel,
                            &client::transport::HostCapabilities::programmable(),
                            move |_, _| async move { reply(&response) },
                        )
                        .await
                }
            };
            let prepared = prepare(&op);
            let exact_response =
                client::exact::Value::parse(&body, client::Limits::default()).unwrap();
            let retained = JsonValue::parse(&body).unwrap();
            for stage in [
                "request-preparation",
                "dispatch-and-client-decode",
                "input-bridge",
                "output-bridge",
                "retained-output-clone",
            ] {
                let samples = sample(|| match stage {
                    "request-preparation" => {
                        black_box(prepare(&op));
                    }
                    "dispatch-and-client-decode" => {
                        black_box(
                            ready(host(prepared.clone(), client::Cancellation::default()))
                                .received_2xx_json_exact()
                                .unwrap(),
                        );
                    }
                    "input-bridge" => {
                        if let Some(body) = value.get("body") {
                            black_box(
                                client::exact::Value::parse(body.text(), client::Limits::default())
                                    .unwrap(),
                            );
                        } else {
                            black_box(&value);
                        }
                    }
                    "output-bridge" => {
                        black_box(JsonValue::parse(exact_response.source()).unwrap());
                    }
                    _ => {
                        black_box(retained.clone());
                    }
                });
                rows.push(serde_json::json!({"workload":"stage","stage":stage,"method":method,"bytes":bytes,"samples_ns":samples}));
            }
            let provider = OpenApi::default().with_transport(host.clone());
            let facade = binding::Invoker::new(&provider);
            let binding = BindingValue {
                content: Some(
                    JsonValue::parse(serde_json::json!({"path":"/p0","method":method}).to_string())
                        .unwrap(),
                ),
                ..Default::default()
            };
            let echo = binding::Invoker::new(Echo(retained.clone()));
            let samples = sample(|| {
                let mut job = echo.invoke(BindingRequest {
                    source: &src,
                    binding: &binding,
                    context: None,
                });
                ready(job.write(value.clone())).unwrap();
                job.close();
                while let Some(v) = ready(job.next()) {
                    black_box(v.unwrap());
                }
                assert!(job.outcome().unwrap().is_ok());
            });
            rows.push(serde_json::json!({"workload":"stage","stage":"checked-binding-lifecycle","method":method,"bytes":bytes,"samples_ns":samples}));
            let mut draft = Document::new();
            let operations = BTreeMap::from([("call".into(), Operation::default())]);
            let sources = BTreeMap::from([("api".into(), src.clone())]);
            let bindings = BTreeMap::from([(
                "call".into(),
                Binding {
                    operation: "call".into(),
                    source: "api".into(),
                    content: binding.content.clone(),
                    ..Default::default()
                },
            )]);
            draft
                .edit(&[
                    Edit::SetOperations { value: &operations },
                    Edit::SetSources { value: &sources },
                    Edit::SetBindings { value: &bindings },
                ])
                .unwrap();
            let document = draft.clone();
            let operations = operation::Invoker::new(
                document
                    .clone()
                    .validate()
                    .unwrap()
                    .conformant()
                    .expect("conformant invocation document"),
                &provider,
            );
            for layer in ["binding", "operation"] {
                let samples = sample(|| {
                    if candidate {
                        if layer == "binding" {
                            let mut job = facade.invoke(BindingRequest {
                                source: &src,
                                binding: &binding,
                                context: None,
                            });
                            ready(job.write(value.clone())).unwrap();
                            job.close();
                            let out = ready(job.next()).unwrap().unwrap();
                            assert_eq!(out.text().len(), bytes);
                            assert!(ready(job.next()).is_none());
                            assert!(job.outcome().unwrap().is_ok());
                            black_box(out);
                        } else {
                            let mut job = operations.invoke(operation::Request::operation("call"));
                            ready(job.write(value.clone())).unwrap();
                            job.close();
                            let out = ready(job.next()).unwrap().unwrap();
                            assert_eq!(out.text().len(), bytes);
                            assert!(ready(job.next()).is_none());
                            assert!(job.outcome().unwrap().is_ok());
                            black_box(out);
                        }
                    } else {
                        let prepared = prepare(&op);
                        let outcome = ready(host(prepared, client::Cancellation::default()));
                        let output = outcome.received_2xx_json_exact().unwrap();
                        assert_eq!(output.source().len(), bytes);
                        black_box(output);
                    }
                });
                rows.push(serde_json::json!({"workload":"invocation","method":method,"bytes":bytes,"payload":envelope.len()+bytes,"layer":layer,"cold":false,"samples_ns":samples}));
            }
            let samples = sample(|| {
                if candidate {
                    let provider = OpenApi::default().with_transport(host.clone());
                    let facade = binding::Invoker::new(provider);
                    let mut job = facade.invoke(BindingRequest {
                        source: &src,
                        binding: &binding,
                        context: None,
                    });
                    ready(job.write(value.clone())).unwrap();
                    job.close();
                    while let Some(v) = ready(job.next()) {
                        black_box(v.unwrap());
                    }
                    assert!(job.outcome().unwrap().is_ok());
                } else {
                    let d = client_doc(&text);
                    let op = d.operations().next().unwrap();
                    let prepared = prepare(&op);
                    black_box(
                        ready(host(prepared, client::Cancellation::default()))
                            .received_2xx_json_exact()
                            .unwrap(),
                    );
                }
            });
            rows.push(serde_json::json!({"workload":"cold-setup-and-invocation","method":method,"bytes":bytes,"source_bytes":text.len(),"samples_ns":samples}));
            // Separate matched contract overhead, not attributed to protocol adaptation.
            draft
                .set_operation_input("call", &JsonValue::parse(r#"{"type":"object"}"#).unwrap())
                .unwrap();
            let doc = draft
                .clone()
                .validate()
                .unwrap()
                .conformant()
                .expect("conformant contract document");
            let evaluator = Arc::new(openbindings_json_schema_evaluator::DefaultEvaluator::new());
            let contracts = doc.value_contracts(evaluator.clone(), ResourceSet::default());
            let ValueContractPreparation::Ready(contract) =
                contracts.prepare("call", Direction::Input)
            else {
                panic!()
            };
            let checked = operation::Invoker::new(doc, &provider)
                .with_evaluator(evaluator, ResourceSet::default());
            // Retain compiled contracts for the warm workload now that core has no cache.
            let prepared_operation = checked.prepare("call").unwrap();
            let samples = sample(|| {
                if candidate {
                    let mut job = prepared_operation.invoke(operation::Attempt::new());
                    ready(job.write(value.clone())).unwrap();
                    job.close();
                    while let Some(v) = ready(job.next()) {
                        black_box(v.unwrap());
                    }
                    assert!(job.outcome().unwrap().is_ok());
                } else {
                    assert!(matches!(contract.validate(&value), ValueOutcome::Satisfies));
                    let prepared = prepare(&op);
                    black_box(
                        ready(host(prepared, client::Cancellation::default()))
                            .received_2xx_json_exact()
                            .unwrap(),
                    );
                }
            });
            rows.push(serde_json::json!({"workload":"validated-invocation","method":method,"bytes":bytes,"payload":envelope.len()+bytes,"samples_ns":samples}));
        }
    }
    println!(
        "{}",
        serde_json::json!({"mode":mode,"edition":version,"rows":rows})
    );
}
