//! Binding content: an exact Paths key and a lowercase method. The value adaptation
//! members are refused before any request, since no evaluator is adopted.
use dynamic_openapi_client::{
    Cancellation, Header, Outcome,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::{Document, JsonValue, Source};
use openbindings_invocation::{BindingRequest, BindingValue, binding, operation};
use openbindings_openapi::{KIND_30, KIND_31, OpenApi};
use serde_json_compat::{Value, json};
use std::{cell::Cell, rc::Rc, sync::Arc};

fn description(version: &str) -> String {
    json!({
        "openapi": version,
        "info": {"title": "Bindings", "version": "1"},
        "servers": [{"url": "https://bindings.example.test"}],
        "paths": {"/tasks/{id}": {"get": {
            "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
            "responses": {"200": {"description": "JSON", "content": {"application/json": {}}}}
        }}}
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
fn host(
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
                            200,
                            vec![Header::new("content-type", "application/json")],
                            Arc::from(&br#"{"task_id":"t-1"}"#[..]),
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
    content: &Value,
    input: &str,
) -> Result<Vec<String>, openbindings_invocation::InvocationError> {
    let binding = BindingValue {
        content: Some(JsonValue::parse(content.to_string()).unwrap()),
        ..Default::default()
    };
    let invoker = binding::Invoker::new(provider);
    let mut job = invoker.invoke(BindingRequest {
        source,
        binding: &binding,
        context: None,
    });
    let _ = job.write(JsonValue::parse(input).unwrap()).await;
    job.close();
    let mut values = vec![];
    while let Some(value) = job.next().await {
        values.push(value?.text().to_owned());
    }
    job.outcome().expect("terminal")?;
    Ok(values)
}

#[tokio::test]
async fn adaptation_members_are_refused_before_any_request() {
    for (kind, version) in [(KIND_31, "3.1.2"), (KIND_30, "3.0.4")] {
        let src = source(kind, version);
        let calls = Rc::new(Cell::new(0));
        let provider = OpenApi::default().with_transport(host(calls.clone()));
        let target = json!({"path": "/tasks/{id}", "method": "get"});
        assert_eq!(
            call(&provider, &src, &target, r#"{"path":{"id":"t-1"}}"#)
                .await
                .unwrap(),
            [r#"{"task_id":"t-1"}"#]
        );
        assert_eq!(calls.get(), 1);
        for (member, expression, input) in [
            ("input", r#"{"path": {"id": id}}"#, r#"{"id":"t-1"}"#),
            // Each is refused even when the input is the native request it would not need.
            ("input", "$", r#"{"path":{"id":"t-1"}}"#),
            ("output", r#"{"id": task_id}"#, r#"{"path":{"id":"t-1"}}"#),
            (
                "failure",
                r#"$status = 404 ? {"missing": true}"#,
                r#"{"path":{"id":"t-1"}}"#,
            ),
        ] {
            let mut content = target.clone();
            content[member] = json!(expression);
            let error = call(&provider, &src, &content, input).await.unwrap_err();
            assert_eq!(error.code(), "ERR_REFUSED", "{member}");
            assert!(error.data().is_none());
        }
        let mut all = target.clone();
        all["input"] = json!("$");
        all["output"] = json!("$");
        all["failure"] = json!("$");
        assert_eq!(
            call(&provider, &src, &all, r#"{"path":{"id":"t-1"}}"#)
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
        assert_eq!(calls.get(), 1, "no adaptation reaches the host");

        // The same refusal through a manually authored document and the operation invoker.
        let mut adapted = target.clone();
        adapted["output"] = json!(r#"{"id": task_id}"#);
        for (binding, accepted) in [(&target, true), (&adapted, false)] {
            let saved = json!({
                "openbindings": "0.2.0",
                "operations": {"getTask": {}},
                "sources": {"api": {"kind": kind, "content": {"document": description(version)}}},
                "bindings": {"getTask": {"operation": "getTask", "source": "api", "content": binding}}
            })
            .to_string();
            let invoker = operation::Invoker::new(
                Document::parse(saved)
                    .unwrap()
                    .validate()
                    .unwrap()
                    .conformant()
                    .expect("conformant authored document"),
                &provider,
            );
            let before = calls.get();
            let mut job = invoker.invoke(operation::Request::operation("getTask"));
            let _ = job
                .write(JsonValue::parse(r#"{"path":{"id":"t-1"}}"#).unwrap())
                .await;
            job.close();
            let first = job.next().await.unwrap();
            if accepted {
                assert_eq!(first.unwrap().text(), r#"{"task_id":"t-1"}"#);
                assert_eq!(calls.get(), before + 1);
            } else {
                assert_eq!(first.unwrap_err().code(), "ERR_REFUSED");
                assert_eq!(calls.get(), before);
            }
        }
    }
}

#[tokio::test]
async fn content_names_an_exact_path_and_a_lowercase_method() {
    for (kind, version) in [(KIND_31, "3.1.2"), (KIND_30, "3.0.4")] {
        let src = source(kind, version);
        let calls = Rc::new(Cell::new(0));
        let provider = OpenApi::default().with_transport(host(calls.clone()));
        for content in [
            json!(null),
            json!("/tasks/{id} get"),
            json!({}),
            json!({"path": "/tasks/{id}"}),
            json!({"method": "get"}),
            json!({"path": "/tasks/{id}", "method": "GET"}),
            json!({"path": "/tasks/{taskId}", "method": "get"}),
            json!({"path": "/tasks/t-1", "method": "get"}),
            json!({"path": "/tasks/{id}", "method": "post"}),
            json!({"path": "/tasks/{id}", "method": ["get"]}),
            json!({"path": "/tasks/{id}", "method": "get", "operationId": "getTask"}),
            json!({"path": "/tasks/{id}", "method": "get", "parameters": {}}),
        ] {
            assert_eq!(
                call(&provider, &src, &content, r#"{"path":{"id":"t-1"}}"#)
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_REFUSED",
                "{content}"
            );
        }
        let duplicate = BindingValue {
            content: Some(
                JsonValue::parse(r#"{"path":"/tasks/{id}","method":"get","method":"get"}"#)
                    .unwrap(),
            ),
            ..Default::default()
        };
        let invoker = binding::Invoker::new(&provider);
        let mut job = invoker.invoke(BindingRequest {
            source: &src,
            binding: &duplicate,
            context: None,
        });
        let _ = job
            .write(JsonValue::parse(r#"{"path":{"id":"t-1"}}"#).unwrap())
            .await;
        job.close();
        assert_eq!(job.next().await.unwrap().unwrap_err().code(), "ERR_REFUSED");
        assert_eq!(calls.get(), 0);
    }
}
