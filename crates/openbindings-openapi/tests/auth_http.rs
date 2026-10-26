//! Real loopback wire evidence with manufactured credentials and a no-follow host: the
//! adapter follows redirects itself, keeping the credential on its origin only.
use dynamic_openapi_client::{Cancellation, transport::PreparedRequest};
use openbindings::{JsonValue, Source};
use openbindings_invocation::{BindingRequest, BindingValue, binding};
use openbindings_openapi::OpenApi;
use std::io::{Read, Write};
use std::net::TcpListener;

/// Read one request head and return it with its `Authorization` field values.
fn request(listener: &TcpListener) -> (std::net::TcpStream, String, Vec<String>) {
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
    let auth = head
        .lines()
        .filter(|l| l.to_lowercase().starts_with("authorization:"))
        .map(|l| l.split_once(':').unwrap().1.trim().to_owned())
        .collect();
    (socket, head, auth)
}

#[tokio::test]
async fn authenticated_wire_and_status_effects() {
    for (kind, text) in [
        (
            openbindings_openapi::KIND_31,
            include_str!("fixtures/auth/service-3.1.2.json"),
        ),
        (
            openbindings_openapi::KIND_30,
            include_str!("fixtures/auth/service-3.0.4.json"),
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        // Another port is another origin.
        let other = TcpListener::bind("127.0.0.1:0").unwrap();
        let elsewhere = format!("http://{}/elsewhere", other.local_addr().unwrap());
        let body = r#"{"n":9007199254740993}"#;
        let server = std::thread::spawn(move || {
            let bearer = vec!["Bearer wire-fixture-token".to_owned()];
            for status in [200, 401, 403, 302] {
                let (mut socket, head, auth) = request(&listener);
                assert!(head.starts_with("GET /items HTTP/1.1\r\n"));
                assert_eq!(auth, bearer);
                write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nLocation: /moved\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            // The same origin keeps the credential; another origin never receives it.
            let (mut socket, head, auth) = request(&listener);
            assert!(head.starts_with("GET /moved HTTP/1.1\r\n"));
            assert_eq!(auth, bearer);
            write!(socket,"HTTP/1.1 307 Fixture\r\nLocation: {elsewhere}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            let (mut socket, head, auth) = request(&other);
            assert!(head.starts_with("GET /elsewhere HTTP/1.1\r\n"));
            assert!(auth.is_empty());
            write!(socket,"HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            6
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
            if status == 401 || status == 403 {
                // No adaptation maps a non-2xx: it fails with no data.
                let error = call.next().await.unwrap().unwrap_err();
                assert_eq!(error.code(), "ERR_EXECUTION_FAILED");
                assert!(error.data().is_none());
            } else {
                // The 302 is followed, through a 307 to another origin, to a 200.
                assert_eq!(call.next().await.unwrap().unwrap().text(), body);
                assert!(call.next().await.is_none());
                call.outcome().unwrap().unwrap();
            }
        }
        assert_eq!(server.join().unwrap(), 6);
    }
}
