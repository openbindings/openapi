//! Server lists at scale: a list that many operations inherit is analyzed and charged to
//! `result_bytes` once, and a large list or a template with many variables stays usable.
#[path = "support/wire.rs"]
mod wire;
use openbindings_openapi::{KIND_30, KIND_31, OpenApi, Options, SERVER_POINT_30, SERVER_POINT_31};
use openbindings_source_inspection::Inspector;
use serde_json_compat::{Value, json};
use wire::{Seen, call, host, source};

const KINDS: [(&str, &str, &str); 2] = [
    (KIND_31, "3.1.2", SERVER_POINT_31),
    (KIND_30, "3.0.4", SERVER_POINT_30),
];

/// `operations` operations `GET /r{n}`, all inheriting the document's `servers`.
fn document(version: &str, servers: Value, operations: usize) -> Value {
    let paths: serde_json_compat::Map<String, Value> = (0..operations)
        .map(|n| {
            (
                format!("/r{n}"),
                json!({"get": {"responses": {"204": {"description": "empty"}}}}),
            )
        })
        .collect();
    json!({
        "openapi": version,
        "info": {"title": "Server lists", "version": "1"},
        "servers": servers,
        "paths": paths
    })
}

#[tokio::test]
async fn a_large_list_that_many_operations_inherit_is_charged_once() {
    for (kind, version, point) in KINDS {
        // 5,000 distinct servers, each declared twice alike, and one URL declared again
        // with other variables.
        let mut servers: Vec<Value> = (0..5_000)
            .map(|i| json!({"url": format!("https://s{i}.example.test")}))
            .collect();
        servers.extend(servers.clone());
        servers.push(json!({
            "url": "https://s7.example.test",
            "variables": {"unused": {"default": "x"}}
        }));
        let src = source(kind, &document(version, json!(servers), 8), None);
        // The list retains about 600 KiB. Charged once for each of the eight operations,
        // it would spend this budget twice over.
        let mut options = Options::default();
        options.result_bytes = 2 << 20;
        let inspection = Inspector::new(OpenApi::new(options.clone()).unwrap())
            .inspect(&src)
            .await
            .unwrap();
        assert_eq!(inspection.targets.len(), 8);
        assert!(inspection.targets.iter().all(|t| t.operation.is_some()));
        // Charged once, it still counts against a budget smaller than itself.
        options.result_bytes = 256 << 10;
        let error = Inspector::new(OpenApi::new(options).unwrap())
            .inspect(&src)
            .await
            .unwrap_err();
        assert_eq!(
            error.code(),
            "openbindings.openapi.result-limit",
            "{version}"
        );
        // Any declared URL can be chosen, except the one declared with other variables.
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        for (url, target) in [
            (
                "https://s4999.example.test",
                Some("https://s4999.example.test/r7"),
            ),
            (
                "https://s0.example.test",
                Some("https://s0.example.test/r7"),
            ),
            ("https://s7.example.test", None),
        ] {
            let sent = seen.borrow().len();
            let context = json!({"configuration": {point: {"url": url}}});
            let result = call(&provider, &src, "/r7", "get", &["{}"], Some(&context)).await;
            match target {
                Some(target) => {
                    result.unwrap();
                    assert_eq!(seen.borrow()[sent].target, target);
                }
                None => {
                    assert_eq!(result.unwrap_err().code(), "ERR_REFUSED", "{version}");
                    assert_eq!(seen.borrow().len(), sent);
                }
            }
        }
    }
}

#[tokio::test]
async fn a_template_with_many_variables_is_analyzed_in_one_pass() {
    const VARIABLES: usize = 20_000;
    for (kind, version, _) in KINDS {
        let mut url = "https://{v0}.example.test".to_owned();
        let mut variables = serde_json_compat::Map::new();
        for i in 0..VARIABLES {
            if i > 0 {
                url.push_str(&format!("/{{v{i}}}"));
            }
            variables.insert(format!("v{i}"), json!({"default": "d", "enum": ["d", "e"]}));
        }
        let servers = json!([{"url": url, "variables": variables}]);
        let src = source(kind, &document(version, servers, 4), None);
        let inspection = Inspector::new(OpenApi::default())
            .inspect(&src)
            .await
            .unwrap();
        assert_eq!(inspection.targets.len(), 4);
        assert!(
            inspection.targets.iter().all(|t| t.operation.is_some()),
            "{version}"
        );
    }
}
