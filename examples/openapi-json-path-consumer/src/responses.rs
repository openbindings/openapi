//! Responses and redirects through the records service's hand-authored OBI. What a call
//! returns is decided by the response received, never by the description's response
//! declarations, and each redirect the adapter follows is a separate host call. The
//! suite runs natively and in the Wasm build, so the adapter's response and redirect code
//! is exercised in both.
use crate::{call, invoker};
use dynamic_openapi_client::{
    Cancellation, Header, Outcome,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::JsonValue;
use openbindings_invocation::operation;
use openbindings_openapi::{KIND_30, KIND_31, OpenApi};
use std::{cell::RefCell, collections::VecDeque, future::Future, pin::Pin, rc::Rc, sync::Arc};

const RECORD: &str = "https://crud.example.test/records/box-7";
const JSON: &[(&str, &str)] = &[("content-type", "application/json")];
/// The PNG signature: octets that are not text.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A response the service sends: status, header fields and content.
struct Reply(u16, &'static [(&'static str, &'static str)], &'static [u8]);

/// What one request carried to the service.
#[derive(Debug, PartialEq)]
struct Sent {
    method: String,
    target: String,
    content_type: Option<String>,
    authorization: Option<String>,
    body: Option<String>,
}
/// A request without credentials; JSON content when it has a body.
fn sent(method: &str, target: &str, body: Option<&str>) -> Sent {
    Sent {
        method: method.into(),
        target: target.into(),
        content_type: body.map(|_| "application/json".into()),
        authorization: None,
        body: body.map(str::to_owned),
    }
}

/// One call: the operation and its native request, the replies the service sends in
/// turn, what the call returns and what the service receives.
struct Case {
    operation: &'static str,
    input: String,
    replies: Vec<Reply>,
    result: Result<Option<&'static str>, &'static str>,
    sent: Vec<Sent>,
}

type Host = Box<dyn Fn(PreparedRequest, Cancellation) -> Pin<Box<dyn Future<Output = Outcome>>>>;

/// A service that answers each request with the next reply, recording what it received.
fn service(replies: Vec<Reply>, sent: Rc<RefCell<Vec<Sent>>>) -> Host {
    let replies = Rc::new(RefCell::new(VecDeque::from(replies)));
    Box::new(move |request, cancel| {
        let Reply(status, headers, content) = replies
            .borrow_mut()
            .pop_front()
            .expect("one reply for every request the adapter sends");
        let sent = sent.clone();
        Box::pin(async move {
            request
                .invoke(
                    cancel,
                    &HostCapabilities::programmable(),
                    move |wire, _| async move {
                        let header = |name: &str| {
                            wire.headers()
                                .iter()
                                .find(|h| h.name().eq_ignore_ascii_case(name))
                                .map(|h| String::from_utf8(h.value().to_vec()).unwrap())
                        };
                        sent.borrow_mut().push(Sent {
                            method: wire.method().as_str().to_owned(),
                            target: wire.target().to_owned(),
                            content_type: header("content-type"),
                            authorization: header("authorization"),
                            body: wire.body().map(|b| String::from_utf8(b.to_vec()).unwrap()),
                        });
                        let mut response = ResponseInput::new(
                            status,
                            headers.iter().map(|(n, v)| Header::new(*n, v)).collect(),
                            Arc::from(content),
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
    })
}

/// Run each case through the records service and check what the call returned and every
/// request the service received. Returns how many requests reached the service.
pub async fn suite(v30: bool) -> usize {
    let (kind, manual) = if v30 {
        (KIND_30, include_str!("crud-fixtures/manual-3.0.4.obi.json"))
    } else {
        (KIND_31, include_str!("crud-fixtures/manual-3.1.2.obi.json"))
    };
    // The bearer token is granted to the records service's own origin only.
    let context = JsonValue::parse(
        serde_json::json!({
            "credentials": {"Token": "crud-token"},
            "configuration": {
                format!("{kind}.security"): {"origins": {"Token": ["https://crud.example.test"]}}
            }
        })
        .to_string(),
    )
    .unwrap();
    let read = || r#"{"path":{"id":"box-7"}}"#.to_owned();
    let record = || sent("GET", RECORD, None);
    let created = r#"{"n":9007199254740993}"#;
    let revised = r#"{"label":"revised"}"#;
    // The description declares JSON for every response it lists; the response received
    // decides.
    let cases = [
        // Text media become a string.
        Case {
            operation: "read",
            input: read(),
            replies: vec![Reply(
                200,
                &[("content-type", "text/plain; charset=utf-8")],
                "雪 is snow".as_bytes(),
            )],
            result: Ok(Some(r#""雪 is snow""#)),
            sent: vec![record()],
        },
        // Other media, and no `Content-Type` at all, become Base64 of the octets.
        Case {
            operation: "read",
            input: read(),
            replies: vec![Reply(200, &[("content-type", "image/png")], PNG)],
            result: Ok(Some(r#""iVBORw0KGgo=""#)),
            sent: vec![record()],
        },
        Case {
            operation: "read",
            input: read(),
            replies: vec![Reply(200, &[], PNG)],
            result: Ok(Some(r#""iVBORw0KGgo=""#)),
            sent: vec![record()],
        },
        // A repeated member name keeps its last occurrence, with its exact number.
        Case {
            operation: "read",
            input: read(),
            replies: vec![Reply(
                200,
                JSON,
                br#"{"id":"box-7","n":1,"n":9007199254740993}"#,
            )],
            result: Ok(Some(r#"{"id":"box-7","n":9007199254740993}"#)),
            sent: vec![record()],
        },
        // No content emits no value.
        Case {
            operation: "read",
            input: read(),
            replies: vec![Reply(200, &[], b"")],
            result: Ok(None),
            sent: vec![record()],
        },
        // A redirect is followed with a second host call.
        Case {
            operation: "read",
            input: read(),
            replies: vec![
                Reply(302, &[("location", "/records/box-8")], b""),
                Reply(200, JSON, br#"{"id":"box-8"}"#),
            ],
            result: Ok(Some(r#"{"id":"box-8"}"#)),
            sent: vec![
                record(),
                sent("GET", "https://crud.example.test/records/box-8", None),
            ],
        },
        // 303 turns the POST into a GET without content.
        Case {
            operation: "create",
            input: format!(r#"{{"body":{created}}}"#),
            replies: vec![
                Reply(303, &[("location", "/records/box-7")], b""),
                Reply(200, JSON, br#"{"id":"box-7","n":9007199254740993}"#),
            ],
            result: Ok(Some(r#"{"id":"box-7","n":9007199254740993}"#)),
            sent: vec![
                sent("POST", "https://crud.example.test/records", Some(created)),
                record(),
            ],
        },
        // 307 repeats the PATCH and its content; the bearer token stays on its origin.
        Case {
            operation: "edit",
            input: format!(r#"{{"path":{{"id":"box-7"}},"body":{revised}}}"#),
            replies: vec![
                Reply(
                    307,
                    &[("location", "https://mirror.example.test/records/box-7")],
                    b"",
                ),
                Reply(202, &[], b""),
            ],
            result: Ok(None),
            sent: vec![
                Sent {
                    authorization: Some("Bearer crud-token".into()),
                    ..sent("PATCH", RECORD, Some(revised))
                },
                sent(
                    "PATCH",
                    "https://mirror.example.test/records/box-7",
                    Some(revised),
                ),
            ],
        },
        // A redirect without `Location` is the final response: unsuccessful, no data.
        Case {
            operation: "read",
            input: read(),
            replies: vec![Reply(302, &[], b"")],
            result: Err("ERR_EXECUTION_FAILED"),
            sent: vec![record()],
        },
        // An https to http redirect is not followed; that 3xx is the final response.
        Case {
            operation: "read",
            input: read(),
            replies: vec![Reply(
                301,
                &[("location", "http://crud.example.test/records/box-7")],
                b"",
            )],
            result: Err("ERR_EXECUTION_FAILED"),
            sent: vec![record()],
        },
    ];
    let mut entries = 0;
    for case in cases {
        let received = Rc::new(RefCell::new(Vec::new()));
        let provider = OpenApi::default().with_transport(service(case.replies, received.clone()));
        let result = call(
            &invoker(manual, provider),
            operation::Request::operation(case.operation).with_context(&context),
            &case.input,
        )
        .await;
        let label = format!("{} {}", case.operation, case.input);
        assert_eq!(
            result.as_ref().map(Option::as_deref).map_err(|e| e.code()),
            case.result,
            "{label}"
        );
        let received = received.take();
        assert_eq!(received, case.sent, "{label}");
        entries += received.len();
    }
    entries
}
