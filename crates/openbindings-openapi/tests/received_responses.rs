//! The received response decides, and declarations never change how a response is read.
//! A final 2xx with complete evidence succeeds whether or not it is declared. No content
//! emits no value; otherwise its `Content-Type` decides the one value: JSON media the JSON
//! value, text a string, anything else (or no `Content-Type`) a Base64 string. Codings
//! that remain on the body fail, and nothing from a header reaches the value.
use dynamic_openapi_client::{
    Cancellation, Header,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::{JsonValue, Source};
use openbindings_interface_synthesis::{
    CoverageStatus, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_invocation::{BindingRequest, BindingValue, binding};
use openbindings_openapi::{KIND_30, KIND_31, OpenApi};
use serde_json_compat::json;
use std::{cell::RefCell, rc::Rc, sync::Arc};

const KINDS: [(&str, &str); 2] = [(KIND_31, "3.1.2"), (KIND_30, "3.0.4")];
const FAILED: &str = "ERR_EXECUTION_FAILED";

fn source(kind: &str, version: &str) -> Source {
    let document = json!({
        "openapi": version,
        "info": {"title": "Received responses", "version": "1"},
        "servers": [{"url": "https://responses.example.test"}],
        "paths": {
            "/json": {"get": {"responses": {"200": {"description": "JSON",
                "content": {"application/json": {"schema": {"type": "object"}}}}}}},
            "/undeclared": {"get": {"responses": {"204": {"description": "empty"}}}},
            "/text": {"get": {"responses": {"200": {"description": "text",
                "content": {"text/plain": {"schema": {"type": "string"}}}}}}},
            "/headers": {"get": {"responses": {"200": {"description": "with headers",
                "headers": {"X-Total-Count": {"schema": {"type": "integer"}}},
                "content": {"application/json": {}}}}}},
            "/problem": {"get": {"responses": {"default": {"description": "problem",
                "content": {"application/problem+json": {}}}}}}
        }
    });
    Source {
        kind: kind.into(),
        content: Some(
            JsonValue::parse(json!({"document": document.to_string()}).to_string()).unwrap(),
        ),
        ..Default::default()
    }
}

/// One response as a host delivers it.
#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, Vec<u8>)>,
    body: Vec<u8>,
    state: BodyState,
    provenance: Provenance,
    delivery: HeaderDelivery,
}
impl Reply {
    fn new(status: u16, body: impl AsRef<[u8]>) -> Self {
        Self {
            status,
            headers: vec![],
            body: body.as_ref().to_vec(),
            state: BodyState::Complete,
            provenance: Provenance::Wire,
            delivery: HeaderDelivery::Complete,
        }
    }
    fn typed(status: u16, media: &str, body: impl AsRef<[u8]>) -> Self {
        Self::new(status, body).header("content-type", media)
    }
    fn header(mut self, name: &'static str, value: impl AsRef<[u8]>) -> Self {
        self.headers.push((name, value.as_ref().to_vec()));
        self
    }
    fn provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }
}

/// The request headers each host entry saw, lowercased.
type Seen = Rc<RefCell<Vec<Vec<String>>>>;

/// Invoke the GET binding at `path` with an empty native request; the values emitted as
/// exact text, or the terminal error code.
async fn respond(
    src: &Source,
    path: &str,
    reply: &Reply,
    seen: &Seen,
) -> Result<Vec<String>, String> {
    let reply = reply.clone();
    let recorded = seen.clone();
    let provider =
        OpenApi::default().with_transport(move |request: PreparedRequest, cancel: Cancellation| {
            let reply = reply.clone();
            let recorded = recorded.clone();
            async move {
                request
                    .invoke(
                        cancel,
                        &HostCapabilities::programmable(),
                        move |wire, _| async move {
                            recorded.borrow_mut().push(
                                wire.headers()
                                    .iter()
                                    .map(|h| h.name().to_ascii_lowercase())
                                    .collect(),
                            );
                            let mut response = ResponseInput::new(
                                reply.status,
                                reply
                                    .headers
                                    .iter()
                                    .map(|(name, value)| Header::new(*name, value))
                                    .collect(),
                                Arc::from(reply.body.as_slice()),
                                reply.state,
                                reply.provenance,
                            );
                            response.header_delivery = reply.delivery;
                            TransportResult {
                                upload: UploadState::Complete,
                                response: Some(response),
                                error_detail: None,
                            }
                        },
                    )
                    .await
            }
        });
    let binding = BindingValue {
        content: Some(
            JsonValue::parse(json!({"path": path, "method": "get"}).to_string()).unwrap(),
        ),
        ..Default::default()
    };
    let invoker = binding::Invoker::new(provider);
    let mut job = invoker.invoke(BindingRequest {
        source: src,
        binding: &binding,
        context: None,
    });
    let _ = job.write(JsonValue::parse("{}").unwrap()).await;
    job.close();
    let mut values = Vec::new();
    while let Some(value) = job.next().await {
        match value {
            Ok(value) => values.push(value.text().to_owned()),
            Err(error) => {
                assert!(values.is_empty(), "an error precedes every value");
                return Err(error.code().to_owned());
            }
        }
    }
    job.outcome()
        .expect("terminal")
        .map(|()| values)
        .map_err(|e| e.code().to_owned())
}

/// Check each reply at `path` under both kinds: `Some(text)` is the one exact value, an
/// empty string no value, and `None` an unsuccessful completion.
async fn expect(path: &str, cases: &[(Reply, Option<&str>)]) {
    for (kind, version) in KINDS {
        let src = source(kind, version);
        for (reply, expected) in cases {
            let seen = Seen::default();
            let result = respond(&src, path, reply, &seen).await;
            let expected = match expected {
                Some("") => Ok(vec![]),
                Some(text) => Ok(vec![(*text).to_owned()]),
                None => Err(FAILED.to_owned()),
            };
            assert_eq!(
                result,
                expected,
                "{kind} {path} {} {:?} {:?}",
                reply.status,
                reply
                    .headers
                    .iter()
                    .map(|(n, v)| (n, String::from_utf8_lossy(v)))
                    .collect::<Vec<_>>(),
                String::from_utf8_lossy(&reply.body)
            );
            assert_eq!(seen.borrow().len(), 1, "one request");
        }
    }
}

/// The JSON string a text becomes.
fn string(text: &str) -> String {
    serde_json_compat::to_string(text).unwrap()
}

#[tokio::test]
async fn json_media_become_the_exact_json_value() {
    let body = r#"{"n":9007199254740993,"price":1.50,"big":1e400,"label":"雪"}"#;
    let cases: Vec<_> = [
        "application/json",
        "Application/JSON",
        "application/json; charset=utf-8",
        "application/json;charset=\"UTF-8\"; profile=x",
        "application/problem+json",
        "application/vnd.acme.parcel+json; version=2",
        "text/vnd.acme+JSON",
    ]
    .into_iter()
    .map(|media| (Reply::typed(200, media, body), Some(body)))
    .collect();
    expect("/json", &cases).await;
    // Insignificant whitespace inside a value that needs no rebuild stays as delivered.
    let spaced = "{ \"a\" : [ 1 , 2 ] }";
    expect(
        "/json",
        &[
            (
                Reply::typed(200, "application/json", format!(" \n{spaced}\n ")),
                Some(spaced),
            ),
            (Reply::typed(200, "application/json", "null"), Some("null")),
            (Reply::typed(200, "application/json", "\"\""), Some("\"\"")),
        ],
    )
    .await;
}

#[tokio::test]
async fn text_media_become_a_string() {
    let text = "héllo \"world\"\n\ttab\u{1}";
    let cases: Vec<_> = [
        "text/plain",
        "TEXT/PLAIN",
        "text/html; charset=utf-8",
        "text/csv;charset=\"UTF-8\"",
        "text/json",
        // Any other type with a charset parameter is text too.
        "application/xml; charset=utf-8",
        "application/octet-stream; charset=Utf-8",
    ]
    .into_iter()
    .map(|media| (Reply::typed(200, media, text), Some(string(text))))
    .collect();
    let cases: Vec<_> = cases
        .iter()
        .map(|(reply, value)| (reply.clone(), value.as_deref()))
        .collect();
    expect("/text", &cases).await;
    let null = string("null");
    expect(
        "/text",
        &[(Reply::typed(200, "text/plain", "null"), Some(null.as_str()))],
    )
    .await;
}

#[tokio::test]
async fn other_media_and_a_missing_content_type_become_base64() {
    let octets = [0x00u8, 0xFF, 0xFE, b'f', b'o', b'o'];
    let encoded = "\"AP/+Zm9v\"";
    let mut cases: Vec<_> = [
        "application/octet-stream",
        "image/png",
        "application/xml",
        "application/x-ndjson",
        "multipart/mixed; boundary=x",
        "application/jsonx",
    ]
    .into_iter()
    .map(|media| (Reply::typed(200, media, octets), Some(encoded)))
    .collect();
    // No `Content-Type` is read as `application/octet-stream`, with no sniffing: even a
    // body that looks like JSON is octets.
    cases.push((Reply::new(200, octets), Some(encoded)));
    cases.push((Reply::new(200, "{\"a\":1}"), Some("\"eyJhIjoxfQ==\"")));
    for (input, output) in [
        ("f", "\"Zg==\""),
        ("fo", "\"Zm8=\""),
        ("foobar", "\"Zm9vYmFy\""),
    ] {
        cases.push((
            Reply::typed(200, "application/octet-stream", input),
            Some(output),
        ));
    }
    expect("/json", &cases).await;
}

#[tokio::test]
async fn content_type_must_be_one_parseable_media_type() {
    let body = r#"{"a":1}"#;
    expect(
        "/json",
        &[
            (
                Reply::typed(200, "application/json", body)
                    .header("content-type", "application/json"),
                None,
            ),
            (
                Reply::typed(200, "text/plain", body).header("Content-Type", "application/json"),
                None,
            ),
            (
                Reply::typed(200, "application/json, text/plain", body),
                None,
            ),
            (Reply::typed(200, "application/json; charset", body), None),
            (
                Reply::typed(200, "application/json; charset=\"utf-8", body),
                None,
            ),
            (Reply::typed(200, "application/json; a=b c", body), None),
            (Reply::typed(200, "json", body), None),
            (Reply::typed(200, "application/", body), None),
            (Reply::typed(200, "application /json", body), None),
            (Reply::typed(200, "", body), None),
            (Reply::typed(200, "application/json;", body), Some(body)),
        ],
    )
    .await;
}

#[tokio::test]
async fn text_charsets_decode_utf8_and_us_ascii_only() {
    let ascii = string("plain ascii");
    let accented = string("é");
    expect(
        "/text",
        &[
            (
                Reply::typed(200, "text/plain", "é"),
                Some(accented.as_str()),
            ),
            (
                Reply::typed(200, "text/plain; charset=UTF-8", "é"),
                Some(accented.as_str()),
            ),
            (
                Reply::typed(200, "text/plain; charset=us-ascii", "plain ascii"),
                Some(ascii.as_str()),
            ),
            (
                Reply::typed(200, "text/plain; charset=US-ASCII", "plain ascii"),
                Some(ascii.as_str()),
            ),
            // US-ASCII holds no octet above 0x7F.
            (
                Reply::typed(200, "text/plain; charset=us-ascii", b"caf\xE9"),
                None,
            ),
            (
                Reply::typed(200, "text/plain; charset=us-ascii", "café"),
                None,
            ),
            // Text that is not valid UTF-8.
            (Reply::typed(200, "text/plain", b"caf\xE9"), None),
            (
                Reply::typed(200, "text/plain; charset=utf-8", b"\xC3"),
                None,
            ),
            // Other charsets are not decoded.
            (
                Reply::typed(200, "text/plain; charset=iso-8859-1", "plain ascii"),
                None,
            ),
            (
                Reply::typed(200, "text/plain; charset=utf-16", "plain ascii"),
                None,
            ),
            (
                Reply::typed(200, "text/plain; charset=utf8", "plain ascii"),
                None,
            ),
            (
                Reply::typed(200, "application/xml; charset=windows-1252", "<a/>"),
                None,
            ),
            // A repeated charset is ambiguous.
            (
                Reply::typed(
                    200,
                    "text/plain; charset=utf-8; charset=utf-8",
                    "plain ascii",
                ),
                None,
            ),
        ],
    )
    .await;
}

#[tokio::test]
async fn a_leading_byte_order_mark_is_not_part_of_the_value() {
    let bom = "\u{FEFF}";
    let text = string("text");
    let inner_bom = string("\u{FEFF}text");
    expect(
        "/json",
        &[
            (
                Reply::typed(200, "application/json", format!("{bom}{{\"a\":1}}")),
                Some("{\"a\":1}"),
            ),
            (
                Reply::typed(200, "application/json", format!("{bom}{bom}{{}}")),
                None,
            ),
            (
                Reply::typed(200, "application/json", format!(" {bom}{{}}")),
                None,
            ),
            (
                Reply::typed(200, "text/plain", format!("{bom}text")),
                Some(text.as_str()),
            ),
            (
                Reply::typed(200, "text/plain; charset=utf-8", format!("{bom}{bom}text")),
                Some(inner_bom.as_str()),
            ),
            (Reply::typed(200, "text/plain", bom), Some("\"\"")),
            // A byte order mark is no US-ASCII text.
            (
                Reply::typed(200, "text/plain; charset=us-ascii", format!("{bom}text")),
                None,
            ),
            // Octets keep every byte.
            (
                Reply::typed(200, "application/octet-stream", bom),
                Some("\"77u/\""),
            ),
        ],
    )
    .await;
}

#[tokio::test]
async fn duplicate_member_names_take_the_last_occurrence_at_every_depth() {
    expect(
        "/json",
        &[
            (
                Reply::typed(
                    200,
                    "application/json",
                    r#"{"a":1,"b":{"c":1,"c":2.50,"d":[{"e":1,"e":1e400}]},"a":9007199254740993}"#,
                ),
                Some(r#"{"b":{"c":2.50,"d":[{"e":1e400}]},"a":9007199254740993}"#),
            ),
            // Names compare by their decoded text, and the kept occurrence keeps its spelling.
            (
                Reply::typed(200, "application/json", r#"{"a":1,"a":-0.0}"#),
                Some(r#"{"a":-0.0}"#),
            ),
            // Only the containers holding duplicates are rebuilt.
            (
                Reply::typed(
                    200,
                    "application/json",
                    r#"[ {"x":1,"x":2}, { "y" : 3 }, {"z":[1,{"z":2,"z":3}]} ]"#,
                ),
                Some(r#"[{"x":2},{ "y" : 3 },{"z":[1,{"z":3}]}]"#),
            ),
            (
                Reply::typed(
                    200,
                    "application/json",
                    r#"{"k":{"a":1},"k":{"b":2,"b":3}}"#,
                ),
                Some(r#"{"k":{"b":3}}"#),
            ),
            (
                Reply::typed(
                    200,
                    "application/json",
                    r#"{"a":1,"b":2,"a":3,"b":4,"a":5}"#,
                ),
                Some(r#"{"b":4,"a":5}"#),
            ),
        ],
    )
    .await;
}

#[tokio::test]
async fn invalid_json_and_unpaired_surrogate_escapes_fail() {
    let mut cases = vec![
        (
            Reply::typed(200, "application/json", r#""😀""#),
            Some(r#""😀""#),
        ),
        (
            Reply::typed(200, "application/json", r#""\\ud800""#),
            Some(r#""\\ud800""#),
        ),
    ];
    for body in [
        r#""\ud800""#,
        r#""\udc00""#,
        r#"["\ud83dA"]"#,
        r#"{"\udfff":1}"#,
        r#"{"a":"x\ud83d"}"#,
        "{",
        r#"{"a":}"#,
        "nul",
        "1 2",
        "[1,]",
        "{\"a\":1,}",
    ] {
        cases.push((Reply::typed(200, "application/json", body), None));
    }
    cases.push((Reply::typed(200, "application/json", b"\"caf\xE9\""), None));
    expect("/json", &cases).await;
}

#[tokio::test]
async fn empty_responses_emit_no_value_and_content_on_205_fails() {
    expect(
        "/undeclared",
        &[
            (Reply::new(204, ""), Some("")),
            (Reply::typed(204, "application/json", ""), Some("")),
            // HTTP framing gives a 204 no content: delivered bytes there fail.
            (Reply::typed(204, "application/json", "null"), None),
            (Reply::new(205, ""), Some("")),
            (Reply::typed(205, "application/json", "null"), None),
            (Reply::typed(205, "text/plain", "reset"), None),
            (Reply::new(200, ""), Some("")),
            (Reply::typed(201, "application/json", ""), Some("")),
            (Reply::typed(202, "text/plain", ""), Some("")),
            (
                Reply::typed(200, "application/json, text/plain", ""),
                Some(""),
            ),
        ],
    )
    .await;
}

#[tokio::test]
async fn an_empty_200_declared_as_json_emits_no_value() {
    expect(
        "/json",
        &[
            (Reply::typed(200, "application/json", ""), Some("")),
            (Reply::new(200, ""), Some("")),
        ],
    )
    .await;
}

#[tokio::test]
async fn every_2xx_succeeds_declared_or_not() {
    let cases: Vec<_> = [200, 201, 202, 203, 206, 207, 226, 299]
        .into_iter()
        .map(|status| (Reply::typed(status, "application/json", "[1]"), Some("[1]")))
        .collect();
    expect("/undeclared", &cases).await;
    // No success declaration excludes an operation: non-JSON success media, declared
    // response headers and a problem `default` are all callable.
    expect(
        "/problem",
        &[(
            Reply::typed(200, "application/problem+json", "{\"ok\":true}"),
            Some("{\"ok\":true}"),
        )],
    )
    .await;
    let synthesized = Synthesizer::new(OpenApi::default())
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &[SynthesisSource {
                name: None,
                source: &source(KIND_31, "3.1.2"),
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let entries = synthesized.coverage().entries();
    assert!(
        entries
            .iter()
            .all(|e| e.status != CoverageStatus::ImplementationUnsupported)
    );
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.status == CoverageStatus::Represented)
            .count(),
        5
    );
}

#[tokio::test]
async fn a_declared_json_200_that_arrives_as_text_is_a_string() {
    let text = string(r#"{"a":1}"#);
    expect(
        "/json",
        &[(
            Reply::typed(200, "text/plain", r#"{"a":1}"#),
            Some(text.as_str()),
        )],
    )
    .await;
    // And a declared text response that arrives as JSON is the JSON value.
    expect(
        "/text",
        &[(
            Reply::typed(200, "application/json", r#"{"a":1}"#),
            Some(r#"{"a":1}"#),
        )],
    )
    .await;
}

#[tokio::test]
async fn codings_that_remain_on_the_body_fail() {
    let json = |coding: &str, provenance| {
        Reply::typed(200, "application/json", "[1]")
            .header("content-encoding", coding)
            .provenance(provenance)
    };
    let mut cases = Vec::new();
    for provenance in [Provenance::Wire, Provenance::Unknown] {
        cases.extend([
            (json("gzip", provenance), None),
            (json("identity, gzip", provenance), None),
            (json("br", provenance), None),
            (json("x-unknown", provenance), None),
            (json("identity", provenance), Some("[1]")),
            (json("Identity, ,identity", provenance), Some("[1]")),
            (
                json("identity", provenance).header("content-encoding", "gzip"),
                None,
            ),
            // Zero octets still carry the coding, which cannot be removed.
            (
                Reply::new(200, "")
                    .header("content-encoding", "gzip")
                    .provenance(provenance),
                None,
            ),
            // A 204 has no content to carry one.
            (
                Reply::new(204, "")
                    .header("content-encoding", "gzip")
                    .provenance(provenance),
                Some(""),
            ),
        ]);
    }
    // A host that removed the codings, or whose platform owns them, says so: its
    // delivered bytes are read as they are.
    for provenance in [Provenance::Decoded, Provenance::HostDelivered] {
        cases.extend([
            (json("gzip", provenance), Some("[1]")),
            (json("br, gzip", provenance), Some("[1]")),
            (json("identity", provenance), Some("[1]")),
            (
                Reply::typed(200, "text/plain", "plain")
                    .header("content-encoding", "gzip")
                    .provenance(provenance),
                Some("\"plain\""),
            ),
            (
                Reply::new(200, "")
                    .header("content-encoding", "gzip")
                    .provenance(provenance),
                Some(""),
            ),
        ]);
    }
    for provenance in [
        Provenance::Wire,
        Provenance::Decoded,
        Provenance::Unknown,
        Provenance::HostDelivered,
    ] {
        cases.push((
            Reply::typed(200, "application/json", "[1]").provenance(provenance),
            Some("[1]"),
        ));
    }
    expect("/json", &cases).await;
}

#[tokio::test]
async fn the_adapter_adds_no_accept_encoding_header() {
    for (kind, version) in KINDS {
        let seen = Seen::default();
        let src = source(kind, version);
        respond(
            &src,
            "/json",
            &Reply::typed(200, "application/json", "[1]"),
            &seen,
        )
        .await
        .unwrap();
        assert!(
            !seen.borrow()[0]
                .iter()
                .any(|name| name == "accept-encoding"),
            "{kind}"
        );
    }
}

#[tokio::test]
async fn headers_never_reach_the_value() {
    let with_headers = |reply: Reply| {
        reply
            .header("x-total-count", "9007199254740993")
            .header("location", "https://responses.example.test/records/7")
            .header("link", "<https://responses.example.test/p2>; rel=\"next\"")
            .header("x-json", "{\"injected\":true}")
            .header("set-cookie", "session=abc")
    };
    for path in ["/headers", "/json", "/undeclared"] {
        expect(
            path,
            &[
                (
                    with_headers(Reply::typed(200, "application/json", "{\"body\":1}")),
                    Some("{\"body\":1}"),
                ),
                (with_headers(Reply::new(201, "")), Some("")),
                (with_headers(Reply::new(204, "")), Some("")),
                (
                    with_headers(Reply::typed(200, "text/plain", "body")),
                    Some("\"body\""),
                ),
            ],
        )
        .await;
    }
}

#[tokio::test]
async fn incomplete_evidence_never_succeeds_for_any_media() {
    let mut cases = Vec::new();
    for media in [
        Some("application/json"),
        Some("text/plain"),
        None,
        Some("image/png"),
    ] {
        let reply = match media {
            Some(media) => Reply::typed(200, media, "[1]"),
            None => Reply::new(200, "[1]"),
        };
        for state in [BodyState::Truncated, BodyState::Failed] {
            let mut truncated = reply.clone();
            truncated.state = state;
            cases.push((truncated, None));
        }
        for delivery in [HeaderDelivery::Unspecified, HeaderDelivery::Incomplete] {
            let mut partial = reply.clone();
            partial.delivery = delivery;
            cases.push((partial, None));
        }
    }
    let mut empty = Reply::new(204, "");
    empty.delivery = HeaderDelivery::Unspecified;
    cases.push((empty, None));
    expect("/json", &cases).await;
}
