//! Real loopback wire evidence with manufactured credentials and a no-follow host.
use dynamic_openapi_client::{Cancellation, transport::PreparedRequest};
use openbindings::{JsonValue, Source};
use openbindings_invocation::{BindingRequest, BindingValue, binding};
use openbindings_openapi::OpenApi;
use std::io::{Read, Write};
#[tokio::test]
async fn authenticated_wire_and_status_effects() {
    for (kind, text) in [
        (
            openbindings_openapi::EXPERIMENTAL_KIND,
            include_str!("fixtures/auth/service-3.1.2.json"),
        ),
        (
            openbindings_openapi::EXPERIMENTAL_KIND_30,
            include_str!("fixtures/auth/service-3.0.4.json"),
        ),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for status in [200, 401, 403, 302] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut buf = vec![];
                loop {
                    let mut b = [0];
                    socket.read_exact(&mut b).unwrap();
                    buf.push(b[0]);
                    if buf.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    assert!(buf.len() < 65536);
                }
                let head = String::from_utf8(buf).unwrap();
                assert!(head.starts_with("GET /items HTTP/1.1\r\n"));
                let auth: Vec<_> = head
                    .lines()
                    .filter(|l| l.to_lowercase().starts_with("authorization:"))
                    .collect();
                assert_eq!(auth.len(), 1);
                assert_eq!(
                    auth[0].split_once(':').unwrap().1.trim(),
                    "Bearer wire-fixture-token"
                );
                let body = r#"{"n":9007199254740993}"#;
                write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nLocation: https://never-follow.example.test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            4
        });
        let src=Source{kind:kind.into(),content:Some(JsonValue::parse(serde_json_compat::json!({"document":text.replace("https://api.example.test",&origin)}).to_string()).unwrap()),..Default::default()};
        let context=JsonValue::parse(serde_json_compat::json!({"credentials":{"ServiceToken":"wire-fixture-token"},"configuration":{format!("{kind}.security"):{"origins":{"ServiceToken":[origin]}}}}).to_string()).unwrap();
        let client = dynamic_openapi_client_reqwest::Client::new().unwrap();
        let p = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancel: Cancellation| {
                let client = client.clone();
                async move {
                    dynamic_openapi_client_reqwest::advanced::execute(&client, &request, cancel)
                        .await
                }
            },
        );
        let invoker = binding::Invoker::new(p);
        let b = BindingValue {
            content: Some(JsonValue::parse(r#"{"path":"/items","method":"get"}"#).unwrap()),
            ..Default::default()
        };
        for status in [200, 401, 403, 302] {
            let mut call = invoker.invoke(BindingRequest {
                source: &src,
                binding: &b,
                context: Some(&context),
            });
            call.write(JsonValue::parse("{}").unwrap()).await.unwrap();
            call.close();
            if status == 200 {
                assert_eq!(
                    call.next().await.unwrap().unwrap().text(),
                    r#"{"n":9007199254740993}"#
                );
                assert!(call.next().await.is_none());
            } else {
                assert_eq!(
                    call.next().await.unwrap().unwrap_err().code(),
                    format!("{kind}.http-status")
                );
            }
        }
        assert_eq!(server.join().unwrap(), 4);
    }
}
