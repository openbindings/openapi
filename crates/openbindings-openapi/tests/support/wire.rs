//! A recording host that answers every request with an empty 204, and a call helper
//! that drives one binding invocation to its terminal outcome.
#![allow(dead_code)]
use dynamic_openapi_client::{
    Cancellation, Outcome,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::{JsonValue, Source};
use openbindings_invocation::{BindingRequest, BindingValue, InvocationError, binding};
use serde_json_compat::{Value, json};
use std::{cell::RefCell, pin::Pin, rc::Rc, sync::Arc};

/// What reached the wire for one request.
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub method: String,
    pub target: String,
    /// Lowercased names with their values, in order.
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}
impl Wire {
    pub fn header(&self, name: &str) -> Option<&str> {
        let mut values = self.headers.iter().filter(|(n, _)| n == name);
        let value = values.next().map(|(_, v)| v.as_str());
        assert!(values.next().is_none(), "one {name} header at most");
        value
    }
}
pub type Seen = Rc<RefCell<Vec<Wire>>>;

pub fn host(
    seen: Seen,
) -> impl Fn(PreparedRequest, Cancellation) -> Pin<Box<dyn Future<Output = Outcome>>> {
    move |request, cancellation| {
        let seen = seen.clone();
        Box::pin(async move {
            request
                .invoke(
                    cancellation,
                    &HostCapabilities::programmable(),
                    move |wire, _| async move {
                        seen.borrow_mut().push(Wire {
                            method: wire.method().as_str().to_owned(),
                            target: wire.target().to_owned(),
                            headers: wire
                                .headers()
                                .iter()
                                .map(|h| {
                                    (
                                        h.name().to_ascii_lowercase(),
                                        String::from_utf8(h.value().to_vec()).unwrap(),
                                    )
                                })
                                .collect(),
                            body: wire.body().map(|b| String::from_utf8(b.to_vec()).unwrap()),
                        });
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

pub fn value(v: &Value) -> JsonValue {
    JsonValue::parse(v.to_string()).unwrap()
}

/// A source embedding `document`, at `location` when one is given.
pub fn source(kind: &str, document: &Value, location: Option<&str>) -> Source {
    let mut content = json!({"document": document});
    if let Some(location) = location {
        content["location"] = json!(location);
    }
    Source {
        kind: kind.into(),
        content: Some(value(&content)),
        ..Default::default()
    }
}

/// Invoke one binding with the given input values, then close input. Every response
/// here is an empty 204, so a successful call emits no value.
pub async fn call<P: binding::BindingInvoker>(
    provider: &P,
    source: &Source,
    path: &str,
    method: &str,
    inputs: &[&str],
    context: Option<&Value>,
) -> Result<(), InvocationError> {
    let binding = BindingValue {
        content: Some(value(&json!({"path": path, "method": method}))),
        ..Default::default()
    };
    let context = context.map(value);
    let invoker = binding::Invoker::new(provider);
    let mut job = invoker.invoke(BindingRequest {
        source,
        binding: &binding,
        context: context.as_ref(),
    });
    for input in inputs {
        let _ = job.write(JsonValue::parse(input).unwrap()).await;
    }
    job.close();
    if let Some(output) = job.next().await {
        output?;
        panic!("an empty response emits no value");
    }
    job.outcome().expect("terminal")
}

/// The challenge carried by a `CONTEXT_REQUIRED` error, as plain JSON.
pub fn challenge(error: &InvocationError) -> Value {
    assert_eq!(error.code(), "CONTEXT_REQUIRED");
    serde_json_compat::from_str(error.data().expect("challenge data").text()).unwrap()
}
