use dynamic_openapi_client::{
    BodyState, Cancellation, DeliveredBody, Header, HeaderDelivery, HostCapabilities, Outcome,
    PreparedRequest, Provenance, ResponseInput, TransportResult, UploadState,
};
use openbindings::{DocumentBuilder, JsonValue, ParsedDocument, WorkControl};
use openbindings_invocation::{BindingRequest, BindingValue, InvocationError, binding, operation};
use openbindings_openapi::{OpenApi, SERVER_POINT};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};
fn json(t: &str) -> JsonValue {
    JsonValue::parse(t).unwrap()
}
fn document() -> ParsedDocument {
    ParsedDocument::parse(include_str!("fixtures/r1/parcels.obi.json")).unwrap()
}
fn outcome_input(status: u16, body: &str, state: BodyState) -> TransportResult {
    let mut response = ResponseInput::new(
        status,
        if status == 204 {
            vec![]
        } else {
            vec![Header::new("content-type", "application/json")]
        },
        DeliveredBody {
            bytes: Arc::from(body.as_bytes()),
            state,
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
fn host(
    count: Rc<Cell<usize>>,
    status: u16,
    body: &'static str,
    state: BodyState,
) -> impl Fn(PreparedRequest, Cancellation) -> std::pin::Pin<Box<dyn Future<Output = Outcome>>> {
    move |request: PreparedRequest, cancellation: Cancellation| {
        let count = count.clone();
        Box::pin(async move {
            request
                .invoke(
                    cancellation,
                    &HostCapabilities::programmable(),
                    move |_, _| async move {
                        count.set(count.get() + 1);
                        outcome_input(status, body, state)
                    },
                )
                .await
        })
    }
}
async fn invoke<P: binding::BindingInvoker>(
    provider: P,
    input: Vec<JsonValue>,
    context: Option<&JsonValue>,
) -> Result<Vec<JsonValue>, InvocationError> {
    let draft = DocumentBuilder::from_json(document().value()).unwrap();
    let source = &draft.sources.as_ref().unwrap()["authoredApi"];
    let binding = BindingValue::from(&draft.bindings.as_ref().unwrap()["healthBinding"]);
    let facade = binding::Invoker::new(provider);
    let mut job = facade.invoke(BindingRequest {
        source,
        binding: &binding,
        context,
    });
    for value in input {
        let _ = job.write(value).await;
    }
    job.close();
    let mut values = vec![];
    while let Some(value) = job.next().await {
        match value {
            Ok(v) => values.push(v),
            Err(error) => {
                assert!(values.is_empty(), "error must precede every output");
                return Err(error);
            }
        }
    }
    job.outcome().unwrap()?;
    Ok(values)
}
#[tokio::test]
async fn caller_close_and_cardinality_precede_effects() {
    let count = Rc::new(Cell::new(0));
    let provider =
        OpenApi::default().with_transport(host(count.clone(), 204, "", BodyState::Complete));
    let invoker = operation::Invoker::new(document(), &provider);
    let mut job = invoker.invoke(operation::Request::operation("health"));
    job.write(json("{}")).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(5), job.next())
            .await
            .is_err()
    );
    assert_eq!(count.get(), 0);
    job.close();
    assert!(job.next().await.is_none());
    assert!(job.outcome().unwrap().is_ok());
    assert_eq!(count.get(), 1);
    for inputs in [
        vec![],
        vec![json("{}"), json("{}")],
        vec![json("null")],
        vec![json(r#"{"parameters":{"query":{"typo":"x"}}}"#)],
    ] {
        assert_eq!(
            invoke(&provider, inputs, None).await.unwrap_err().code(),
            "ERR_REFUSED"
        );
        assert_eq!(count.get(), 1);
    }
}
#[tokio::test]
async fn failed_or_incomplete_responses_never_emit_success() {
    for (status, body, state, code) in [
        (200, "", BodyState::Complete, "ERR_EXECUTION_FAILED"),
        (200, "{", BodyState::Complete, "ERR_EXECUTION_FAILED"),
        (200, "null", BodyState::Truncated, "ERR_EXECUTION_FAILED"),
        (204, "null", BodyState::Complete, "ERR_EXECUTION_FAILED"),
        (
            302,
            "",
            BodyState::Complete,
            openbindings_openapi::HTTP_STATUS_ERROR,
        ),
        (
            404,
            "not json",
            BodyState::Complete,
            openbindings_openapi::HTTP_STATUS_ERROR,
        ),
    ] {
        let count = Rc::new(Cell::new(0));
        let provider = OpenApi::default().with_transport(host(count.clone(), status, body, state));
        assert_eq!(
            invoke(provider, vec![json("{}")], None)
                .await
                .unwrap_err()
                .code(),
            code
        );
        assert_eq!(count.get(), 1);
    }
}
#[tokio::test]
async fn destination_context_is_membership_and_new_attempt() {
    let doc = ParsedDocument::parse(include_str!("fixtures/r1/destinations.obi.json")).unwrap();
    let count = Rc::new(Cell::new(0));
    let observed = Rc::new(std::cell::RefCell::new(Vec::new()));
    let host_observed = observed.clone();
    let calls = count.clone();
    let provider =
        OpenApi::default().with_transport(move |request: PreparedRequest, cancel: Cancellation| {
            let calls = calls.clone();
            let observed = host_observed.clone();
            async move {
                request
                    .invoke(
                        cancel,
                        &HostCapabilities::programmable(),
                        move |request, _| async move {
                            calls.set(calls.get() + 1);
                            observed.borrow_mut().push(request.target);
                            outcome_input(204, "", BodyState::Complete)
                        },
                    )
                    .await
            }
        });
    let invoker = operation::Invoker::new(doc, provider);
    for (context, code) in [
        (json("{}"), "CONTEXT_REQUIRED"),
        (
            json(&format!(r#"{{"configuration":{{"{SERVER_POINT}":17}}}}"#)),
            "ERR_REFUSED",
        ),
        (
            json(&format!(
                r#"{{"configuration":{{"{SERVER_POINT}":"https://evil.invalid"}}}}"#
            )),
            "ERR_REFUSED",
        ),
    ] {
        let mut job =
            invoker.invoke(operation::Request::operation("health").with_context(&context));
        job.write(json("{}")).await.unwrap();
        job.close();
        let err = job.next().await.unwrap().unwrap_err();
        assert_eq!(err.code(), code);
        assert_eq!(err.context_required().is_some(), code == "CONTEXT_REQUIRED");
        assert_eq!(count.get(), 0);
    }
    let context = json(&format!(
        r#"{{"configuration":{{"{SERVER_POINT}":"https://beta.example.invalid/api"}}}}"#
    ));
    let mut job = invoker.invoke(operation::Request::operation("health").with_context(&context));
    job.write(json("{}")).await.unwrap();
    job.close();
    assert!(job.next().await.is_none());
    assert_eq!(count.get(), 1);
    assert_eq!(
        observed.borrow()[0],
        "https://beta.example.invalid/api/health"
    );
}
#[tokio::test]
async fn pending_local_host_cancellation_and_drop_release_owners() {
    struct Pending(Rc<Cell<usize>>);
    impl Drop for Pending {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }
    for abandon in [false, true] {
        let pending = Rc::new(Cell::new(0));
        let tokens = Rc::new(std::cell::RefCell::new(Vec::new()));
        let registrations = pending.clone();
        let recorded = tokens.clone();
        let provider = OpenApi::default().with_transport(
            move |_request: PreparedRequest, cancel: Cancellation| {
                let registrations = registrations.clone();
                recorded.borrow_mut().push(cancel.clone());
                async move {
                    registrations.set(registrations.get() + 1);
                    let _guard = Pending(registrations);
                    std::future::pending::<Outcome>().await
                }
            },
        );
        let invoker = operation::Invoker::new(document(), provider);
        let control = WorkControl::new();
        let mut job =
            invoker.invoke_with_control(operation::Request::operation("health"), &control);
        job.write(json("{}")).await.unwrap();
        job.close();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), job.next())
                .await
                .is_err()
        );
        assert_eq!(pending.get(), 1);
        if !abandon {
            control.cancel();
            assert_eq!(
                job.next().await.unwrap().unwrap_err().code(),
                "ERR_CANCELLED"
            );
        }
        drop(job);
        assert_eq!(pending.get(), 0);
        assert!(tokens.borrow()[0].is_cancelled());
    }
}

#[tokio::test]
async fn exact_integer_parameters_agree_with_the_operation_schema() {
    use openbindings::{ResourceSet, Source};
    use openbindings_interface_synthesis::{SynthesisRequest, SynthesisSource, Synthesizer};
    use openbindings_json_schema_evaluator::DefaultEvaluator;
    let source = Source {kind:openbindings_openapi::EXPERIMENTAL_KIND.into(), content:Some(json(&serde_json_compat::json!({"document":r#"{"openapi":"3.1.2","info":{"title":"Numbers","version":"1"},"servers":[{"url":"https://numbers.invalid"}],"paths":{"/n":{"get":{"parameters":[{"name":"n","in":"query","schema":{"type":"integer"}}],"responses":{"204":{"description":"empty"}}}}}}"#}).to_string())),..Default::default()};
    let calls = Rc::new(Cell::new(0));
    let provider =
        OpenApi::default().with_transport(host(calls.clone(), 204, "", BodyState::Complete));
    let inputs = [SynthesisSource {
        name: None,
        source: &source,
    }];
    let generated = Synthesizer::new(&provider)
        .synthesize(&SynthesisRequest {
            sources: &inputs,
            ..Default::default()
        })
        .await
        .unwrap();
    let parsed = ParsedDocument::parse(generated.parsed().value().text()).unwrap();
    let draft = DocumentBuilder::from_json(parsed.value()).unwrap();
    let (name, binding) = draft.bindings.as_ref().unwrap().iter().next().unwrap();
    let value = BindingValue::from(binding);
    let direct = binding::Invoker::new(&provider);
    let operations = operation::Invoker::new(parsed.clone(), &provider)
        .with_evaluator(
            Arc::new(DefaultEvaluator::default()),
            ResourceSet::default(),
        )
        .unwrap();
    for (token, accepted) in [
        ("10e-1", true),
        ("10.0e-1", true),
        ("100.0e-2", true),
        ("100.00e-2", true),
        ("-100.0e-2", true),
        ("9007199254740993", true),
        ("100.1e-2", false),
        ("10.1e-1", false),
    ] {
        let input = json(&format!(r#"{{"parameters":{{"query":{{"n":{token}}}}}}}"#));
        let before = calls.get();
        let mut call = direct.invoke(BindingRequest {
            source: &source,
            binding: &value,
            context: None,
        });
        let _ = call.write(input.clone()).await;
        call.close();
        while call.next().await.is_some() {}
        assert_eq!(call.outcome().unwrap().is_ok(), accepted, "binding {token}");
        let mut call = operations.invoke(operation::Request::operation(name));
        let _ = call.write(input).await;
        call.close();
        while call.next().await.is_some() {}
        assert_eq!(
            call.outcome().unwrap().is_ok(),
            accepted,
            "operation {token}"
        );
        assert_eq!(calls.get() - before, if accepted { 2 } else { 0 });
    }
}

#[tokio::test]
async fn incomplete_headers_and_pre_cancel_are_not_success() {
    for delivery in [HeaderDelivery::Unspecified, HeaderDelivery::Incomplete] {
        let provider = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancel: Cancellation| async move {
                request
                    .invoke(
                        cancel,
                        &HostCapabilities::programmable(),
                        move |_, _| async move {
                            let mut result = outcome_input(200, "null", BodyState::Complete);
                            result.response.as_mut().unwrap().header_delivery = delivery;
                            result
                        },
                    )
                    .await
            },
        );
        assert_eq!(
            invoke(provider, vec![json("{}")], None)
                .await
                .unwrap_err()
                .code(),
            "ERR_EXECUTION_FAILED"
        );
    }
    let calls = Rc::new(Cell::new(0));
    let provider =
        OpenApi::default().with_transport(host(calls.clone(), 204, "", BodyState::Complete));
    let facade = operation::Invoker::new(document(), provider);
    let control = WorkControl::new();
    control.cancel();
    let mut job = facade.invoke_with_control(operation::Request::operation("health"), &control);
    let _ = job.write(json("{}")).await;
    job.close();
    while job.next().await.is_some() {}
    assert_eq!(
        job.outcome().unwrap().as_ref().unwrap_err().code(),
        "ERR_CANCELLED"
    );
    assert_eq!(calls.get(), 0);
}

#[tokio::test]
async fn generic_selection_and_author_contracts_remain_in_the_operation_layer() {
    use openbindings_json_schema_evaluator::DefaultEvaluator;
    let calls = Rc::new(Cell::new(0));
    let provider =
        OpenApi::default().with_transport(host(calls.clone(), 200, "null", BodyState::Complete));
    let mut draft = DocumentBuilder::from_json(document().value()).unwrap();
    let b = draft.bindings.as_ref().unwrap()["healthBinding"].clone();
    draft
        .bindings
        .as_mut()
        .unwrap()
        .insert("other_health".into(), b);
    let facade = operation::Invoker::new(
        ParsedDocument::from_json(draft.to_json().unwrap()),
        &provider,
    );
    let mut job = facade.invoke(operation::Request::operation("health"));
    let _ = job.write(json("{}")).await;
    job.close();
    while job.next().await.is_some() {}
    assert!(job.outcome().unwrap().is_err());
    assert_eq!(calls.get(), 0);
    let mut job = facade.invoke(operation::Request::binding("other_health"));
    job.write(json("{}")).await.unwrap();
    job.close();
    while job.next().await.is_some() {}
    assert!(job.outcome().unwrap().is_ok());
    assert_eq!(calls.get(), 1);
    drop(job);
    for (side, expected_calls) in [("input", 1), ("output", 2)] {
        let mut draft = DocumentBuilder::from_json(document().value()).unwrap();
        if side == "input" {
            draft.operations.get_mut("health").unwrap().input = Some(json("false"));
        } else {
            draft.operations.get_mut("health").unwrap().output = Some(json("false"));
        }
        let facade = operation::Invoker::new(
            ParsedDocument::from_json(draft.to_json().unwrap()),
            &provider,
        )
        .with_evaluator(
            Arc::new(DefaultEvaluator::new()),
            openbindings::ResourceSet::default(),
        )
        .unwrap();
        let mut job = facade.invoke(operation::Request::operation("health"));
        let _ = job.write(json("{}")).await;
        job.close();
        let mut successes = 0;
        while let Some(value) = job.next().await {
            if value.is_ok() {
                successes += 1;
            }
        }
        assert!(job.outcome().unwrap().is_err());
        assert_eq!(successes, 0);
        assert_eq!(calls.get(), expected_calls);
    }
}

#[tokio::test]
async fn required_values_duplicates_media_and_body_budget_are_checked() {
    let count = Rc::new(Cell::new(0));
    let provider =
        OpenApi::default().with_transport(host(count.clone(), 204, "", BodyState::Complete));
    let draft = DocumentBuilder::from_json(document().value()).unwrap();
    let source = &draft.sources.as_ref().unwrap()["authoredApi"];
    let facade = binding::Invoker::new(&provider);
    for (key, input) in [
        ("lookupBinding", "{}"),
        ("createBinding", r#"{"body":{"x":1,"x":2}}"#),
    ] {
        let value = BindingValue::from(&draft.bindings.as_ref().unwrap()[key]);
        let mut job = facade.invoke(BindingRequest {
            source,
            binding: &value,
            context: None,
        });
        let _ = job.write(json(input)).await;
        job.close();
        while job.next().await.is_some() {}
        assert!(job.outcome().unwrap().is_err());
    }
    assert_eq!(count.get(), 0);
    for (limit, accepted) in [(4, true), (3, false)] {
        let mut options = openbindings_openapi::Options::default();
        options.limits.body_bytes = limit;
        let provider = OpenApi::new(options).unwrap().with_transport(host(
            count.clone(),
            204,
            "",
            BodyState::Complete,
        ));
        let facade = binding::Invoker::new(provider);
        let value = BindingValue::from(&draft.bindings.as_ref().unwrap()["createBinding"]);
        let mut job = facade.invoke(BindingRequest {
            source,
            binding: &value,
            context: None,
        });
        let _ = job.write(json(r#"{"body":null}"#)).await;
        job.close();
        while job.next().await.is_some() {}
        assert_eq!(job.outcome().unwrap().is_ok(), accepted);
    }
    assert_eq!(count.get(), 1);
    let provider = OpenApi::default().with_transport(
        |request: PreparedRequest, cancel: Cancellation| async move {
            request
                .invoke(
                    cancel,
                    &HostCapabilities::programmable(),
                    |_, _| async move {
                        let mut result = outcome_input(200, "null", BodyState::Complete);
                        result.response.as_mut().unwrap().headers =
                            vec![Header::new("content-type", "text/plain")];
                        result
                    },
                )
                .await
        },
    );
    assert_eq!(
        invoke(provider, vec![json("{}")], None)
            .await
            .unwrap_err()
            .code(),
        "ERR_EXECUTION_FAILED"
    );
}

#[tokio::test]
async fn required_body_and_parameter_count_refuse_before_host_entry() {
    let text = r#"{"openapi":"3.1.2","info":{"title":"Required","version":"1"},"servers":[{"url":"https://required.invalid"}],"paths":{"/x":{"post":{"parameters":[{"name":"a","in":"query","schema":{"type":"string"}},{"name":"b","in":"query","schema":{"type":"string"}}],"requestBody":{"required":true,"content":{"application/json":{}}},"responses":{"204":{"description":"empty"}}}}}}"#;
    let source = openbindings::Source {
        kind: openbindings_openapi::EXPERIMENTAL_KIND.into(),
        content: Some(json(
            &serde_json_compat::json!({"document":text}).to_string(),
        )),
        ..Default::default()
    };
    let value = BindingValue {
        content: Some(json(r#"{"path":"/x","method":"post"}"#)),
        ..Default::default()
    };
    let calls = Rc::new(Cell::new(0));
    let mut options = openbindings_openapi::Options::default();
    options.limits.reference_steps = 1;
    let facade = binding::Invoker::new(OpenApi::new(options).unwrap().with_transport(host(
        calls.clone(),
        204,
        "",
        BodyState::Complete,
    )));
    for (input, accepted) in [
        ("{}", false),
        (r#"{"body":null,"parameters":{"query":{"a":"one"}}}"#, true),
        (
            r#"{"body":null,"parameters":{"query":{"a":"one","b":"two"}}}"#,
            false,
        ),
    ] {
        let before = calls.get();
        let mut job = facade.invoke(BindingRequest {
            source: &source,
            binding: &value,
            context: None,
        });
        let _ = job.write(json(input)).await;
        job.close();
        while job.next().await.is_some() {}
        assert_eq!(job.outcome().unwrap().is_ok(), accepted);
        assert_eq!(calls.get() - before, usize::from(accepted));
    }
}

#[test]
fn pinned_client_literal_unicode_limitation_is_reproduced_independently() {
    // Preserve the failed FR06 expectation as a client limitation. An encoded
    // path is an additional qualified case, not a substitute for this case.
    let text = r#"{"openapi":"3.1.2","info":{"title":"Unicode","version":"1"},"servers":[{"url":"https://unicode.invalid"}],"paths":{"/雪~s":{"get":{"responses":{"204":{"description":"empty"}}}}}}"#;
    let document =
        dynamic_openapi_client::Document::parse(text.as_bytes(), None, Default::default()).unwrap();
    let error = document
        .operations()
        .next()
        .unwrap()
        .prepare(&Default::default())
        .unwrap_err();
    assert_eq!(
        error.code(),
        dynamic_openapi_client::Code::InvalidDestination
    );
}
