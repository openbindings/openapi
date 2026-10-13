use dynamic_openapi_client::{Cancellation, PreparedRequest};
use openbindings::{JsonValue, ParsedDocument};
use openbindings_invocation::{InvocationError, operation};
use openbindings_openapi::OpenApi;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

pub struct Reply {
    pub method: &'static str,
    pub target: &'static str,
    pub request: Option<&'static str>,
    pub status: u16,
    pub body: &'static str,
}

#[tokio::test]
async fn crud_methods_and_empty_success_over_native_http() {
    for (version, kind) in [
        ("3.0.4", openbindings_openapi::EXPERIMENTAL_KIND_30),
        ("3.1.2", openbindings_openapi::EXPERIMENTAL_KIND),
    ] {
        let (origin, server) = server(vec![
            Reply {
                method: "PUT",
                target: "/records",
                request: Some(r#"{"n":9007199254740993}"#),
                status: 200,
                body: r#"{"n":9007199254740993}"#,
            },
            Reply {
                method: "PATCH",
                target: "/records",
                request: Some("null"),
                status: 202,
                body: "",
            },
            Reply {
                method: "DELETE",
                target: "/records",
                request: None,
                status: 204,
                body: "",
            },
            Reply {
                method: "DELETE",
                target: "/records",
                request: None,
                status: 205,
                body: "",
            },
        ]);
        use serde_json_compat::json;
        let body = json!({"required":true,"content":{"application/json":{"schema":{}}}});
        let source = json!({"openapi":version,"info":{"title":"Native CRUD","version":"1"},"servers":[{"url":origin}],"paths":{"/records":{
            "put":{"requestBody":body,"responses":{"200":{"description":"updated","content":{"application/json":{}}}}},
            "patch":{"requestBody":body,"responses":{"202":{"description":"accepted"}}},
            "delete":{"responses":{"204":{"description":"removed"},"205":{"description":"reset"}}}
        }}});
        let saved = json!({"openbindings":"0.2.0","operations":{"replace":{},"edit":{},"remove":{}},"sources":{"api":{"kind":kind,"content":{"document":source.to_string()}}},"bindings":{
            "replace":{"operation":"replace","source":"api","content":{"path":"/records","method":"put"}},
            "edit":{"operation":"edit","source":"api","content":{"path":"/records","method":"patch"}},
            "remove":{"operation":"remove","source":"api","content":{"path":"/records","method":"delete"}}
        }}).to_string();
        let client = dynamic_openapi_client_reqwest::Client::new().unwrap();
        let provider = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancel: Cancellation| {
                let client = client.clone();
                async move { client.execute(&request, cancel).await }
            },
        );
        let invoker = operation::Invoker::new(ParsedDocument::parse(saved).unwrap(), provider);
        assert_eq!(
            call(&invoker, "replace", r#"{"body":{"n":9007199254740993}}"#)
                .await
                .unwrap()
                .unwrap()
                .text(),
            r#"{"n":9007199254740993}"#
        );
        assert!(
            call(&invoker, "edit", r#"{"body":null}"#)
                .await
                .unwrap()
                .is_none()
        );
        assert!(call(&invoker, "remove", "{}").await.unwrap().is_none());
        assert!(call(&invoker, "remove", "{}").await.unwrap().is_none());
        server.join().unwrap();
    }
}
fn server(replies: Vec<Reply>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        for reply in replies {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let (header, body) = request(&mut socket);
            assert_eq!(
                header.lines().next().unwrap(),
                format!("{} {} HTTP/1.1", reply.method, reply.target)
            );
            match reply.request {
                None => assert!(body.is_empty()),
                Some(expected) => assert_eq!(
                    serde_json_compat::from_slice::<serde_json_compat::Value>(&body).unwrap(),
                    serde_json_compat::from_str::<serde_json_compat::Value>(expected).unwrap()
                ),
            }
            let media = if reply.status == 204 {
                ""
            } else {
                "Content-Type: application/json\r\n"
            };
            write!(
                socket,
                "HTTP/1.1 {} Fixture\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.status,
                media,
                reply.body.len(),
                reply.body
            )
            .unwrap();
        }
    });
    (origin, handle)
}
fn request(socket: &mut TcpStream) -> (String, Vec<u8>) {
    let mut all = Vec::new();
    let end = loop {
        let mut byte = [0];
        socket.read_exact(&mut byte).unwrap();
        all.push(byte[0]);
        if all.ends_with(b"\r\n\r\n") {
            break all.len();
        }
        assert!(all.len() < 65536);
    };
    let head = String::from_utf8(all[..end].to_vec()).unwrap();
    let len = head
        .lines()
        .find_map(|s| {
            s.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let mut body = vec![0; len];
    socket.read_exact(&mut body).unwrap();
    (head, body)
}
fn document(origin: &str) -> ParsedDocument {
    // Recorded harness substitution before both authoring routes. The source text
    // remains a text value; its exact large integer never passes through a JS number.
    ParsedDocument::parse(
        include_str!("fixtures/r1/parcels.obi.json")
            .replace("https://parcel.example.invalid", origin),
    )
    .unwrap()
}
async fn call<P: openbindings_invocation::binding::BindingInvoker>(
    invoker: &operation::Invoker<P>,
    name: &str,
    text: &str,
) -> Result<Option<JsonValue>, InvocationError> {
    let mut call = invoker.invoke(operation::Request::operation(name));
    // A write error means stop sending; the terminal carries the actual failure.
    let _ = call.write(JsonValue::parse(text).unwrap()).await;
    call.close();
    let mut result = None;
    while let Some(value) = call.next().await {
        let value = value?;
        assert!(result.is_none());
        result = Some(value);
    }
    call.outcome().expect("terminal")?;
    Ok(result)
}
#[tokio::test]
async fn authored_bindings_make_real_http_calls_and_preserve_exact_values() {
    let (origin, server) = server(vec![
        Reply {
            method: "GET",
            target: "/api/parcels/box-7?parcelId=alias-9",
            request: None,
            status: 200,
            body: r#"{"parcelId":"box-7","tracking":9007199254740993}"#,
        },
        Reply {
            method: "POST",
            target: "/api/parcels",
            request: Some(r#"{"tracking":9007199254740993,"label":"雪"}"#),
            status: 201,
            body: r#"{"tracking":9007199254740993}"#,
        },
        Reply {
            method: "POST",
            target: "/api/parcels",
            request: None,
            status: 204,
            body: "",
        },
        Reply {
            method: "POST",
            target: "/api/parcels",
            request: Some("null"),
            status: 201,
            body: "null",
        },
        Reply {
            method: "GET",
            target: "/api/parcels/missing",
            request: None,
            status: 404,
            body: r#"{"error":"missing"}"#,
        },
    ]);
    let client = dynamic_openapi_client_reqwest::Client::new().unwrap();
    let provider = OpenApi::default().with_transport(
        move |request: PreparedRequest, cancellation: Cancellation| {
            let client = client.clone();
            async move { client.execute(&request, cancellation).await }
        },
    );
    let invoker = operation::Invoker::new(document(&origin), provider);
    assert_eq!(
        call(
            &invoker,
            "lookup",
            r#"{"parameters":{"path":{"parcelId":"box-7"},"query":{"parcelId":"alias-9"}}}"#
        )
        .await
        .unwrap()
        .unwrap()
        .get("tracking")
        .unwrap()
        .number_text(),
        Some("9007199254740993")
    );
    assert_eq!(
        call(
            &invoker,
            "create",
            r#"{"body":{"tracking":9007199254740993,"label":"雪"}}"#
        )
        .await
        .unwrap()
        .unwrap()
        .get("tracking")
        .unwrap()
        .number_text(),
        Some("9007199254740993")
    );
    assert!(call(&invoker, "create", "{}").await.unwrap().is_none());
    assert_eq!(
        call(&invoker, "create", r#"{"body":null}"#)
            .await
            .unwrap()
            .unwrap()
            .text(),
        "null"
    );
    let error = call(
        &invoker,
        "lookup",
        r#"{"parameters":{"path":{"parcelId":"missing"}}}"#,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), openbindings_openapi::HTTP_STATUS_ERROR);
    assert_eq!(error.data().unwrap().text(), r#"{"status":404}"#);
    server.join().unwrap();
}
#[test]
fn native_provider_future_retains_send() {
    fn send<T: Send>(_: T) {}
    let provider = OpenApi::default().with_transport(|_: PreparedRequest, _: Cancellation| {
        std::future::ready(dynamic_openapi_client::Outcome::not_dispatched(
            dynamic_openapi_client::Diagnostic::new(dynamic_openapi_client::Code::HostCapability),
            false,
            None,
        ))
    });
    let invoker = operation::Invoker::new(document("http://127.0.0.1:1"), provider);
    send(invoker.invoke(operation::Request::operation("health")));
}

#[tokio::test]
async fn synthesized_reloaded_document_repeats_the_independent_wire_call() {
    use openbindings_interface_synthesis::{SynthesisRequest, SynthesisSource, Synthesizer};
    use openbindings_json_schema_evaluator::DefaultEvaluator;
    let (origin, server) = server(vec![
        Reply {
            method: "GET",
            target: "/api/parcels/box-7?parcelId=alias-9",
            request: None,
            status: 200,
            body: r#"{"parcelId":"box-7","tracking":9007199254740993}"#,
        },
        Reply {
            method: "POST",
            target: "/api/parcels",
            request: Some(r#"{"tracking":9007199254740993,"label":"雪"}"#),
            status: 201,
            body: r#"{"tracking":9007199254740993}"#,
        },
    ]);
    let saved = {
        let draft = openbindings::DocumentBuilder::from_json(document(&origin).value()).unwrap();
        let source = &draft.sources.as_ref().unwrap()["authoredApi"];
        let provider = OpenApi::default();
        let sources = [SynthesisSource {
            name: Some("consumerOwnedSource"),
            source,
        }];
        let request = SynthesisRequest {
            sources: &sources,
            name: Some("Saved API"),
            ..Default::default()
        };
        let synth = Synthesizer::new(&provider)
            .synthesize_with_coverage(&request)
            .await
            .unwrap();
        assert!(synth.coverage().strict_eligible());
        assert!(!synth.coverage().fully_represented());
        synth.document().parsed().value().text().to_owned()
    };
    // All authoring owners are gone; a new provider loads a persisted document.
    let parsed = ParsedDocument::parse(saved).unwrap();
    let draft = openbindings::DocumentBuilder::from_json(parsed.value()).unwrap();
    let binding = draft
        .bindings
        .as_ref()
        .unwrap()
        .values()
        .find(|b| {
            b.content
                .as_ref()
                .unwrap()
                .get("path")
                .and_then(openbindings::JsonRef::as_str)
                == Some("/parcels/{parcelId}")
        })
        .unwrap();
    let operation = binding.operation.clone();
    let create = draft
        .bindings
        .as_ref()
        .unwrap()
        .values()
        .find(|b| {
            b.content
                .as_ref()
                .unwrap()
                .get("path")
                .and_then(openbindings::JsonRef::as_str)
                == Some("/parcels")
        })
        .unwrap()
        .operation
        .clone();
    let client = dynamic_openapi_client_reqwest::Client::new().unwrap();
    let provider = OpenApi::default().with_transport(
        move |request: PreparedRequest, cancellation: Cancellation| {
            let client = client.clone();
            async move { client.execute(&request, cancellation).await }
        },
    );
    let invoker = operation::Invoker::new(parsed, provider)
        .with_evaluator(
            std::sync::Arc::new(DefaultEvaluator::new()),
            openbindings::ResourceSet::default(),
        )
        .unwrap();
    let output = call(
        &invoker,
        &operation,
        r#"{"parameters":{"path":{"parcelId":"box-7"},"query":{"parcelId":"alias-9"}}}"#,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        output.get("tracking").unwrap().number_text(),
        Some("9007199254740993")
    );
    let created = call(
        &invoker,
        &create,
        r#"{"body":{"tracking":9007199254740993,"label":"雪"}}"#,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        created.get("tracking").unwrap().number_text(),
        Some("9007199254740993")
    );
    server.join().unwrap();
}

#[tokio::test]
async fn cold_reloaded_mounts_use_supplied_reference_bases_on_real_http() {
    use openbindings_interface_synthesis::{SynthesisRequest, SynthesisSource, Synthesizer};
    use openbindings_json_schema_evaluator::DefaultEvaluator;
    use openbindings_openapi::{EXPERIMENTAL_KIND, Options, ResourceInput};
    fn options() -> Options {
        let mut o = Options::default();
        o.resources = vec![
            ResourceInput::at(
                "https://descriptions.example.invalid/fragments/parts.json",
                include_bytes!("fixtures/r1/nested-parts.json").as_slice(),
            )
            .unwrap(),
            ResourceInput::at(
                "https://descriptions.example.invalid/fragments/parameters.json",
                include_bytes!("fixtures/r1/nested-parameters.json").as_slice(),
            )
            .unwrap(),
        ];
        o
    }
    let (origin, server) = server(vec![
        Reply {
            method: "GET",
            target: "/api/mounted/box-7",
            request: None,
            status: 200,
            body: r#"{"parcelId":"box-7"}"#,
        },
        Reply {
            method: "GET",
            target: "/api/alternate/box-7",
            request: None,
            status: 200,
            body: r#"{"parcelId":"box-7"}"#,
        },
    ]);
    let saved = {
        let text = include_str!("fixtures/r1/nested-entry.json")
            .replace("https://parcel.example.invalid", &origin);
        let source=openbindings::Source{kind:EXPERIMENTAL_KIND.into(),content:Some(JsonValue::parse(serde_json_compat::json!({"document":text,"retrievalUri":"https://descriptions.example.invalid/root.json"}).to_string()).unwrap()),..Default::default()};
        let provider = OpenApi::new(options()).unwrap();
        let sources = [SynthesisSource {
            name: None,
            source: &source,
        }];
        let result = Synthesizer::new(&provider)
            .synthesize(&SynthesisRequest {
                sources: &sources,
                ..Default::default()
            })
            .await
            .unwrap();
        result.parsed().value().text().to_owned()
    };
    let doc = ParsedDocument::parse(&saved).unwrap();
    let draft = openbindings::DocumentBuilder::from_json(doc.value()).unwrap();
    let keys: Vec<_> = ["/mounted/{parcelId}", "/alternate/{parcelId}"]
        .into_iter()
        .map(|path| {
            draft
                .bindings
                .as_ref()
                .unwrap()
                .values()
                .find(|b| b.content.as_ref().unwrap().get("path").unwrap().as_str() == Some(path))
                .unwrap()
                .operation
                .clone()
        })
        .collect();
    let client = dynamic_openapi_client_reqwest::Client::new().unwrap();
    let provider = OpenApi::new(options()).unwrap().with_transport(
        move |request: PreparedRequest, cancel: Cancellation| {
            let client = client.clone();
            async move { client.execute(&request, cancel).await }
        },
    );
    let invoker = operation::Invoker::new(doc, provider)
        .with_evaluator(
            std::sync::Arc::new(DefaultEvaluator::new()),
            openbindings::ResourceSet::default(),
        )
        .unwrap();
    for key in &keys {
        assert_eq!(
            call(
                &invoker,
                key,
                r#"{"parameters":{"path":{"parcelId":"box-7"}}}"#
            )
            .await
            .unwrap()
            .unwrap()
            .get("parcelId")
            .unwrap()
            .as_str(),
            Some("box-7")
        );
    }
    server.join().unwrap();
    let no_resources = OpenApi::default().with_transport(|_: PreparedRequest, _: Cancellation| {
        panic!("missing resource must refuse before host entry");
        #[allow(unreachable_code)]
        std::future::ready(dynamic_openapi_client::Outcome::not_dispatched(
            dynamic_openapi_client::Diagnostic::new(dynamic_openapi_client::Code::HostCapability),
            false,
            None,
        ))
    });
    let invoker = operation::Invoker::new(ParsedDocument::parse(saved).unwrap(), no_resources)
        .with_evaluator(
            std::sync::Arc::new(DefaultEvaluator::new()),
            openbindings::ResourceSet::default(),
        )
        .unwrap();
    assert_eq!(
        call(
            &invoker,
            &keys[0],
            r#"{"parameters":{"path":{"parcelId":"box-7"}}}"#
        )
        .await
        .unwrap_err()
        .code(),
        "ERR_REFUSED"
    );
}

#[tokio::test]
async fn manual_mount_and_renamed_unicode_target_use_literal_locations() {
    use openbindings_openapi::{Options, ResourceInput};
    let (origin, server) = server(vec![
        Reply {
            method: "GET",
            target: "/api/mounted/box-7",
            request: None,
            status: 200,
            body: "null",
        },
        Reply {
            method: "GET",
            target: "/api/%E9%9B%AA~s/box-7?parcelId=alias-9",
            request: None,
            status: 200,
            body: "null",
        },
    ]);
    let doc = ParsedDocument::parse(
        include_str!("fixtures/r1/mounted.obi.json")
            .replace("https://parcel.example.invalid", &origin),
    )
    .unwrap();
    let mut options = Options::default();
    options.resources = vec![
        ResourceInput::at(
            "https://descriptions.example.invalid/parts.json",
            include_bytes!("fixtures/r1/parts.json").as_slice(),
        )
        .unwrap(),
    ];
    let client = dynamic_openapi_client_reqwest::Client::new().unwrap();
    let host = move |request: PreparedRequest, cancel: Cancellation| {
        let client = client.clone();
        async move { client.execute(&request, cancel).await }
    };
    let provider = OpenApi::new(options).unwrap().with_transport(host.clone());
    assert!(
        call(
            &operation::Invoker::new(doc, provider),
            "lookup",
            r#"{"parameters":{"path":{"parcelId":"box-7"}}}"#
        )
        .await
        .unwrap()
        .unwrap()
        .text()
            == "null"
    );
    // These names and the route are authored independently of synthesized naming.
    let text = document(&origin)
        .value()
        .text()
        .replace("/parcels/{parcelId}", "/%E9%9B%AA~s/{parcelId}")
        .replace("lookupBinding", "chosen_binding")
        .replace("authoredApi", "chosen_source")
        .replace("lookup", "chosen_operation");
    let invoker = operation::Invoker::new(
        ParsedDocument::parse(text).unwrap(),
        OpenApi::default().with_transport(host),
    );
    assert!(
        call(
            &invoker,
            "chosen_operation",
            r#"{"parameters":{"path":{"parcelId":"box-7"},"query":{"parcelId":"alias-9"}}}"#
        )
        .await
        .unwrap()
        .unwrap()
        .text()
            == "null"
    );
    assert_eq!(
        call(&invoker, "missing-operation", "{}")
            .await
            .unwrap_err()
            .code(),
        "ERR_OPERATION_NOT_FOUND"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn real_http_redirect_and_post_receipt_cancellation_have_one_effect() {
    use std::sync::mpsc;
    for cancel in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (sent, received) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let (head, _) = request(&mut socket);
            assert!(head.starts_with("POST /api/parcels HTTP/1.1"));
            sent.send(()).unwrap();
            if cancel {
                let _ = wait.recv_timeout(std::time::Duration::from_secs(5));
            } else {
                socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /api/parcels/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err());
        });
        let client = dynamic_openapi_client_reqwest::Client::new().unwrap();
        let provider = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancel: Cancellation| {
                let client = client.clone();
                async move { client.execute(&request, cancel).await }
            },
        );
        let invoker = operation::Invoker::new(document(&origin), provider);
        let control = openbindings::WorkControl::new();
        let mut job =
            invoker.invoke_with_control(operation::Request::operation("create"), &control);
        job.write(JsonValue::parse("{}").unwrap()).await.unwrap();
        job.close();
        if cancel {
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(30), job.next())
                    .await
                    .is_err()
            );
            received
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap();
            control.cancel();
        }
        let error = job.next().await.unwrap().unwrap_err();
        assert_eq!(
            error.code(),
            if cancel {
                "ERR_CANCELLED"
            } else {
                openbindings_openapi::HTTP_STATUS_ERROR
            }
        );
        drop(job);
        let _ = release.send(());
        server.join().unwrap();
    }
}
