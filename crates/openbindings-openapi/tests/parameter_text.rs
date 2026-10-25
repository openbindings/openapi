//! Parameter text: a boolean is written `true` or `false`, and a number as the plain
//! decimal numeral of its exact value, bounded by the client's numeral limit. A JSON body
//! keeps its JSON text exactly.
#[path = "support/wire.rs"]
mod wire;
use openbindings_openapi::{KIND_30, KIND_31, OpenApi, Options};
use serde_json_compat::{Value, json};
use wire::{Seen, call, host, source};

const EDITIONS: [(&str, &str); 2] = [(KIND_31, "3.1.2"), (KIND_30, "3.0.4")];

fn document(version: &str) -> Value {
    let empty = json!({"204": {"description": "empty"}});
    json!({
        "openapi": version,
        "info": {"title": "Parameter text", "version": "1"},
        "servers": [{"url": "https://text.example.test"}],
        "paths": {
            "/values/{p}": {"post": {
                "parameters": [
                    {"name": "p", "in": "path", "required": true, "schema": {"type": "number"}},
                    {"name": "n", "in": "query", "schema": {"type": "number"}},
                    {"name": "i", "in": "query", "schema": {"type": "integer"}},
                    {"name": "b", "in": "query", "schema": {"type": "boolean"}}
                ],
                "requestBody": {"content": {"application/json": {}}},
                "responses": empty
            }}
        }
    })
}

#[tokio::test]
async fn numbers_are_written_as_the_plain_decimal_numeral_of_their_exact_value() {
    for (kind, version) in EDITIONS {
        let src = source(kind, &document(version), None);
        for (token, text) in [
            ("1e2", "100"),
            ("1E+2", "100"),
            ("100.0", "100"),
            ("100", "100"),
            ("1.50", "1.5"),
            ("1E-2", "0.01"),
            ("12.5e-3", "0.0125"),
            ("-0", "0"),
            ("-0.0e3", "0"),
            ("-2.50", "-2.5"),
            ("9007199254740993", "9007199254740993"),
            ("9007199254740993.000", "9007199254740993"),
            ("0.1", "0.1"),
        ] {
            let seen = Seen::default();
            let provider = OpenApi::default().with_transport(host(seen.clone()));
            let input = format!(r#"{{"path":{{"p":{token}}},"query":{{"n":{token}}}}}"#);
            call(&provider, &src, "/values/{p}", "post", &[&input], None)
                .await
                .unwrap();
            let seen = seen.borrow();
            assert_eq!(
                seen[0].target,
                format!("https://text.example.test/values/{text}?n={text}"),
                "{version} {token}"
            );
        }
    }
}

#[tokio::test]
async fn integers_and_booleans_keep_their_checks_and_their_text() {
    for (kind, version) in EDITIONS {
        let src = source(kind, &document(version), None);
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        for (query, text) in [
            (r#"{"i":10e-1}"#, "i=1"),
            (r#"{"i":-100.0e-2}"#, "i=-1"),
            (r#"{"i":2E3}"#, "i=2000"),
            (r#"{"b":true}"#, "b=true"),
            (r#"{"b":false}"#, "b=false"),
        ] {
            let input = format!(r#"{{"path":{{"p":1}},"query":{query}}}"#);
            call(&provider, &src, "/values/{p}", "post", &[&input], None)
                .await
                .unwrap();
            assert_eq!(
                seen.borrow().last().unwrap().target,
                format!("https://text.example.test/values/1?{text}"),
                "{version} {query}"
            );
        }
        let before = seen.borrow().len();
        for query in [
            r#"{"i":1.5}"#,
            r#"{"i":15e-1}"#,
            r#"{"i":"1"}"#,
            r#"{"b":"true"}"#,
            r#"{"b":1}"#,
            r#"{"n":"1e2"}"#,
            r#"{"n":true}"#,
        ] {
            let input = format!(r#"{{"path":{{"p":1}},"query":{query}}}"#);
            assert_eq!(
                call(&provider, &src, "/values/{p}", "post", &[&input], None)
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_REFUSED",
                "{version} {query}"
            );
        }
        assert_eq!(seen.borrow().len(), before);
    }
}

#[tokio::test]
async fn a_numeral_longer_than_the_client_limit_is_refused_before_sending() {
    for (kind, version) in EDITIONS {
        let src = source(kind, &document(version), None);
        // The default limit admits a 256-character numeral and nothing longer.
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        call(
            &provider,
            &src,
            "/values/{p}",
            "post",
            &[r#"{"path":{"p":1e255}}"#],
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            seen.borrow()[0].target,
            format!("https://text.example.test/values/1{}", "0".repeat(255))
        );
        for input in [
            r#"{"path":{"p":1e256}}"#,
            r#"{"path":{"p":1},"query":{"i":1e999999999999}}"#,
            r#"{"path":{"p":1},"query":{"n":1e-999999999999}}"#,
        ] {
            assert_eq!(
                call(&provider, &src, "/values/{p}", "post", &[input], None)
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_REFUSED",
                "{version} {input}"
            );
        }
        assert_eq!(seen.borrow().len(), 1);
        // A configured limit bounds the expanded numeral, sign and point included.
        let mut options = Options::default();
        options.limits.number_conversion_chars = 4;
        let seen = Seen::default();
        let provider = OpenApi::new(options)
            .unwrap()
            .with_transport(host(seen.clone()));
        for (token, accepted) in [
            ("1e3", true),
            ("-1e2", true),
            ("1e-2", true),
            ("1.2300", true),
            ("1e4", false),
            ("-1e3", false),
            ("1e-3", false),
            ("12345", false),
        ] {
            let input = format!(r#"{{"path":{{"p":{token}}}}}"#);
            let result = call(&provider, &src, "/values/{p}", "post", &[&input], None).await;
            assert_eq!(result.is_ok(), accepted, "{version} {token}");
        }
        let targets: Vec<_> = seen.borrow().iter().map(|w| w.target.clone()).collect();
        assert_eq!(
            targets,
            ["1000", "-100", "0.01", "1.23"]
                .map(|text| format!("https://text.example.test/values/{text}"))
        );
    }
}

#[tokio::test]
async fn a_json_body_keeps_its_number_text_exactly() {
    for (kind, version) in EDITIONS {
        let src = source(kind, &document(version), None);
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        call(
            &provider,
            &src,
            "/values/{p}",
            "post",
            &[r#"{"path":{"p":1e2},"body":{"n":1e2,"m":1.50,"z":-0}}"#],
            None,
        )
        .await
        .unwrap();
        let seen = seen.borrow();
        assert_eq!(seen[0].target, "https://text.example.test/values/100");
        assert_eq!(
            seen[0].body.as_deref(),
            Some(r#"{"n":1e2,"m":1.50,"z":-0}"#)
        );
    }
}
