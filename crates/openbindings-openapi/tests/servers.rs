//! Servers: the effective servers are the operation's, else the Path Item's, else the
//! document's, and none means `/`. Relative URLs resolve against `location`; variables
//! take a supplied value, else their default; several servers are chosen at
//! `<kind>.server` by a declared URL.
#[path = "support/wire.rs"]
mod wire;
use openbindings_interface_synthesis::{
    CoverageStatus, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_openapi::{
    KIND_30, KIND_31, OpenApi, SECURITY_POINT_30, SECURITY_POINT_31, SERVER_POINT_30,
    SERVER_POINT_31,
};
use serde_json_compat::{Value, json};
use wire::{Seen, call, challenge, host, source};

const KINDS: [(&str, &str, &str, &str); 2] = [
    (KIND_31, "3.1.2", SERVER_POINT_31, SECURITY_POINT_31),
    (KIND_30, "3.0.4", SERVER_POINT_30, SECURITY_POINT_30),
];
const LOCATION: &str = "https://descriptions.example.test/apis/openapi.json";

fn empty() -> Value {
    json!({"204": {"description": "empty"}})
}
/// One `GET /ping` operation under the given document-level servers, if any.
fn ping(version: &str, servers: Option<Value>) -> Value {
    let mut document = json!({
        "openapi": version,
        "info": {"title": "Servers", "version": "1"},
        "paths": {"/ping": {"get": {"responses": empty()}}}
    });
    if let Some(servers) = servers {
        document["servers"] = servers;
    }
    document
}
fn configured(point: &str, value: Value) -> Value {
    json!({"configuration": {point: value}})
}
async fn targets(
    src: &openbindings::Source,
    path: &str,
    method: &str,
    context: Option<&Value>,
) -> Result<String, openbindings_invocation::InvocationError> {
    let seen = Seen::default();
    let provider = OpenApi::default().with_transport(host(seen.clone()));
    let result = call(&provider, src, path, method, &["{}"], context).await;
    let seen = seen.borrow();
    match result {
        Ok(()) => Ok(seen[0].target.clone()),
        Err(error) => {
            assert!(seen.is_empty(), "a refusal or challenge precedes the host");
            Err(error)
        }
    }
}
async fn unsupported(src: &openbindings::Source, reason: &str) {
    let sources = [SynthesisSource {
        name: None,
        source: src,
    }];
    let result = Synthesizer::new(OpenApi::default())
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &sources,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        result
            .coverage()
            .entries()
            .iter()
            .any(|e| e.source_ref == "/paths/~1ping/get"
                && e.status == CoverageStatus::ImplementationUnsupported
                && e.reason_code.as_deref() == Some(&format!("openbindings.openapi.{reason}")))
    );
    assert_eq!(
        targets(src, "/ping", "get", None).await.unwrap_err().code(),
        "ERR_REFUSED"
    );
}

#[tokio::test]
async fn effective_servers_are_the_operations_else_the_path_items_else_the_documents() {
    for (kind, version, _, _) in KINDS {
        let document = json!({
            "openapi": version,
            "info": {"title": "Precedence", "version": "1"},
            "servers": [{"url": "https://document.example.test/v1"}],
            "paths": {
                "/root": {"get": {"responses": empty()}},
                "/item": {
                    "servers": [{"url": "https://path-item.example.test"}],
                    "get": {"responses": empty()},
                    "put": {
                        "servers": [{"url": "https://operation.example.test/"}],
                        "responses": empty()
                    }
                }
            }
        });
        let src = source(kind, &document, None);
        for (path, method, target) in [
            ("/root", "get", "https://document.example.test/v1/root"),
            ("/item", "get", "https://path-item.example.test/item"),
            // One trailing `/` on the server URL is dropped before the path is appended.
            ("/item", "put", "https://operation.example.test/item"),
        ] {
            assert_eq!(
                targets(&src, path, method, None).await.unwrap(),
                target,
                "{version} {method} {path}"
            );
        }
    }
}

#[tokio::test]
async fn no_servers_mean_one_server_at_the_location_root() {
    for (kind, version, _, _) in KINDS {
        for servers in [None, Some(json!([]))] {
            let document = ping(version, servers.clone());
            let src = source(kind, &document, Some(LOCATION));
            assert_eq!(
                targets(&src, "/ping", "get", None).await.unwrap(),
                "https://descriptions.example.test/ping",
                "{version} {servers:?}"
            );
            // Without a `location`, the implied `/` has nothing to resolve against.
            unsupported(
                &source(kind, &document, None),
                "server-profile-unimplemented",
            )
            .await;
        }
    }
}

#[tokio::test]
async fn an_implied_root_resolves_against_the_document_declaring_the_empty_list() {
    use dynamic_openapi_client::{graph::ReferenceKind, inspect::Edition};
    use openbindings_openapi::{Options, ResourceInput, ResourceLayout};
    use openbindings_source_inspection::Inspector;
    const PIECES: &str = "https://pieces.example.test/shared/routes.json";
    let get = |servers: Option<Value>| {
        let mut operation = json!({"responses": empty()});
        if let Some(servers) = servers {
            operation["servers"] = servers;
        }
        operation
    };
    for (kind, version, _, _) in KINDS {
        let edition = if version.starts_with("3.0") {
            Edition::OpenApi30
        } else {
            Edition::OpenApi31
        };
        // Path Items in another document: one declaring an empty list, one whose operation
        // declares one, and one declaring nothing, which inherits the entry document's.
        let pieces = json!({
            "Declared": {"servers": [], "get": get(None)},
            "Own": {"get": get(Some(json!([])))},
            "Inherits": {"get": get(None)}
        });
        let mut layout = ResourceLayout::new();
        for name in ["/Declared", "/Own", "/Inherits"] {
            layout = layout.protocol(name, ReferenceKind::PathItem, edition);
        }
        let mut options = Options::default();
        options.resources = vec![
            ResourceInput::at(PIECES, pieces.to_string().into_bytes())
                .unwrap()
                .layout(layout),
        ];
        for document_servers in [None, Some(json!([]))] {
            let mut document = json!({
                "openapi": version,
                "info": {"title": "Implied servers", "version": "1"},
                "paths": {
                    "/document": {"get": get(None)},
                    "/operation": {"get": get(Some(json!([])))},
                    "/path-item": {"servers": [], "get": get(None)},
                    "/declared": {"$ref": format!("{PIECES}#/Declared")},
                    "/own": {"$ref": format!("{PIECES}#/Own")},
                    "/inherits": {"$ref": format!("{PIECES}#/Inherits")}
                }
            });
            if let Some(servers) = &document_servers {
                document["servers"] = servers.clone();
            }
            for location in [Some(LOCATION), None] {
                let src = source(kind, &document, location);
                let inspection = Inspector::new(OpenApi::new(options.clone()).unwrap())
                    .inspect(&src)
                    .await
                    .unwrap();
                for (path, declared_elsewhere) in [
                    ("/document", false),
                    ("/operation", false),
                    ("/path-item", false),
                    ("/declared", true),
                    ("/own", true),
                    ("/inherits", false),
                ] {
                    // `/` resolves against the location of the document declaring the empty
                    // list, or the entry document's when none is declared.
                    let origin = if declared_elsewhere {
                        Some("https://pieces.example.test")
                    } else {
                        location.map(|_| "https://descriptions.example.test")
                    };
                    let case = format!("{version} {path} {location:?} {document_servers:?}");
                    let source_ref = format!("/paths/{}/get", path.replace('/', "~1"));
                    let target = inspection
                        .targets
                        .iter()
                        .find(|t| t.source_ref == source_ref)
                        .unwrap();
                    assert_eq!(target.operation.is_some(), origin.is_some(), "{case}");
                    let seen = Seen::default();
                    let provider = OpenApi::new(options.clone())
                        .unwrap()
                        .with_transport(host(seen.clone()));
                    let result = call(&provider, &src, path, "get", &["{}"], None).await;
                    match origin {
                        Some(origin) => {
                            result.unwrap();
                            assert_eq!(
                                seen.borrow()[0].target,
                                format!("{origin}{path}"),
                                "{case}"
                            );
                        }
                        None => {
                            assert_eq!(result.unwrap_err().code(), "ERR_REFUSED", "{case}");
                            assert!(seen.borrow().is_empty(), "{case}");
                        }
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn relative_server_urls_resolve_against_the_location() {
    for (kind, version, point, _) in KINDS {
        for (url, target) in [
            ("/v2", "https://descriptions.example.test/v2/ping"),
            ("v2/", "https://descriptions.example.test/apis/v2/ping"),
            ("..", "https://descriptions.example.test/ping"),
            (
                "//mirror.example.test:8443",
                "https://mirror.example.test:8443/ping",
            ),
        ] {
            let document = ping(version, Some(json!([{"url": url}])));
            assert_eq!(
                targets(
                    &source(kind, &document, Some(LOCATION)),
                    "/ping",
                    "get",
                    None
                )
                .await
                .unwrap(),
                target,
                "{version} {url}"
            );
            unsupported(
                &source(kind, &document, None),
                "server-profile-unimplemented",
            )
            .await;
        }
        // Beside an absolute server, a relative one without a `location` is offered as no
        // choice and refused when named.
        let document = ping(
            version,
            Some(json!([{"url": "/v2"}, {"url": "https://absolute.example.test"}])),
        );
        let src = source(kind, &document, None);
        let error = targets(&src, "/ping", "get", None).await.unwrap_err();
        let requirement = &challenge(&error)["alternatives"][0]["requirements"][0];
        assert_eq!(
            requirement["schema"]["enum"],
            json!(["https://absolute.example.test"])
        );
        assert_eq!(
            targets(
                &src,
                "/ping",
                "get",
                Some(&configured(point, json!({"url": "/v2"})))
            )
            .await
            .unwrap_err()
            .code(),
            "ERR_REFUSED"
        );
        assert_eq!(
            targets(
                &src,
                "/ping",
                "get",
                Some(&configured(
                    point,
                    json!({"url": "https://absolute.example.test"})
                ))
            )
            .await
            .unwrap(),
            "https://absolute.example.test/ping"
        );
        // With a `location`, both resolve.
        let src = source(kind, &document, Some(LOCATION));
        assert_eq!(
            targets(
                &src,
                "/ping",
                "get",
                Some(&configured(point, json!({"url": "/v2"})))
            )
            .await
            .unwrap(),
            "https://descriptions.example.test/v2/ping"
        );
    }
}

#[tokio::test]
async fn variables_take_a_supplied_value_else_their_default() {
    for (kind, version, point, _) in KINDS {
        let document = ping(
            version,
            Some(json!([{
                "url": "https://{region}.example.test/{version}",
                "variables": {
                    "region": {"default": "eu", "enum": ["eu", "us"]},
                    "version": {"default": "v1"},
                    "tenant": {"default": "shared"}
                }
            }])),
        );
        let src = source(kind, &document, None);
        for (variables, target) in [
            (None, "https://eu.example.test/v1/ping"),
            (Some(json!({})), "https://eu.example.test/v1/ping"),
            (
                Some(json!({"region": "us"})),
                "https://us.example.test/v1/ping",
            ),
            (
                Some(json!({"version": "v9"})),
                "https://eu.example.test/v9/ping",
            ),
            (
                Some(json!({"region": "us", "version": "beta"})),
                "https://us.example.test/beta/ping",
            ),
            // A declared variable the URL does not use changes nothing.
            (
                Some(json!({"tenant": "acme"})),
                "https://eu.example.test/v1/ping",
            ),
        ] {
            let context = variables.map(|v| configured(point, json!({"variables": v})));
            assert_eq!(
                targets(&src, "/ping", "get", context.as_ref())
                    .await
                    .unwrap(),
                target,
                "{version} {context:?}"
            );
        }
        // `url` is optional with one server, and names it exactly when given.
        let named = configured(
            point,
            json!({"url": "https://{region}.example.test/{version}", "variables": {"region": "us"}}),
        );
        assert_eq!(
            targets(&src, "/ping", "get", Some(&named)).await.unwrap(),
            "https://us.example.test/v1/ping"
        );
        for variables in [
            json!({"region": "ap"}),
            json!({"region": ""}),
            json!({"zone": "a"}),
            json!({"region": 7}),
            json!({"region": null}),
            json!(["us"]),
            json!("us"),
        ] {
            assert_eq!(
                targets(
                    &src,
                    "/ping",
                    "get",
                    Some(&configured(point, json!({"variables": variables})))
                )
                .await
                .unwrap_err()
                .code(),
                "ERR_REFUSED",
                "{version} {variables}"
            );
        }
    }
}

#[tokio::test]
async fn a_templated_destination_is_the_origin_its_credentials_are_granted_for() {
    for (kind, version, point, security) in KINDS {
        let mut document = ping(
            version,
            Some(json!([{
                "url": "https://{region}.example.test",
                "variables": {"region": {"default": "eu", "enum": ["eu", "us"]}}
            }])),
        );
        document["components"] =
            json!({"securitySchemes": {"Token": {"type": "http", "scheme": "bearer"}}});
        document["security"] = json!([{"Token": []}]);
        let src = source(kind, &document, None);
        let context = |region: Option<&str>, grant: &str| {
            let mut context = json!({
                "credentials": {"Token": "fixture-token"},
                "configuration": {security: {"origins": {"Token": [grant]}}}
            });
            if let Some(region) = region {
                context["configuration"][point] = json!({"variables": {"region": region}});
            }
            context
        };
        assert_eq!(
            targets(
                &src,
                "/ping",
                "get",
                Some(&context(Some("us"), "https://us.example.test"))
            )
            .await
            .unwrap(),
            "https://us.example.test/ping"
        );
        assert_eq!(
            targets(
                &src,
                "/ping",
                "get",
                Some(&context(None, "https://eu.example.test"))
            )
            .await
            .unwrap(),
            "https://eu.example.test/ping"
        );
        // A grant for another destination never covers this one.
        for (region, grant) in [
            (None, "https://us.example.test"),
            (Some("us"), "https://eu.example.test"),
        ] {
            assert_eq!(
                targets(&src, "/ping", "get", Some(&context(region, grant)))
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_REFUSED",
                "{version} {region:?} {grant}"
            );
        }
        // A missing grant is challenged for the chosen destination.
        let mut missing = context(Some("us"), "https://us.example.test");
        missing["configuration"][security] = json!({});
        let error = targets(&src, "/ping", "get", Some(&missing))
            .await
            .unwrap_err();
        let challenge = challenge(&error);
        assert!(
            challenge["target"]
                .as_str()
                .unwrap()
                .contains("https://us.example.test:443")
        );
        assert_eq!(
            challenge["alternatives"][0]["requirements"][0]["examples"],
            Value::Null
        );
        assert_eq!(
            challenge["alternatives"][0]["requirements"][0]["schema"]["examples"],
            json!([["https://us.example.test:443"]])
        );
    }
}

#[tokio::test]
async fn several_servers_are_chosen_by_url_at_the_server_point() {
    for (kind, version, point, _) in KINDS {
        let document = ping(
            version,
            Some(json!([
                {"url": "https://b.example.test"},
                {"url": "https://a.example.test/v1"},
                {"url": "https://b.example.test"}
            ])),
        );
        let src = source(kind, &document, None);
        for context in [
            None,
            Some(json!({})),
            Some(configured(point, json!({}))),
            Some(configured(point, json!({"variables": {}}))),
        ] {
            let error = targets(&src, "/ping", "get", context.as_ref())
                .await
                .unwrap_err();
            let challenge = challenge(&error);
            assert_eq!(
                challenge["alternatives"],
                json!([{"requirements": [{
                    "type": "config.value",
                    "point": point,
                    "path": "/url",
                    "schema": {"type": "string", "enum": ["https://a.example.test/v1", "https://b.example.test"]}
                }]}]),
                "{version} {context:?}"
            );
            assert!(challenge["target"].as_str().unwrap().contains(kind));
        }
        for (url, target) in [
            (
                "https://a.example.test/v1",
                "https://a.example.test/v1/ping",
            ),
            ("https://b.example.test", "https://b.example.test/ping"),
        ] {
            assert_eq!(
                targets(
                    &src,
                    "/ping",
                    "get",
                    Some(&configured(point, json!({"url": url})))
                )
                .await
                .unwrap(),
                target
            );
        }
        for choice in [
            json!("https://a.example.test/v1"),
            json!({"url": "https://a.example.test/v1/"}),
            json!({"url": "HTTPS://A.EXAMPLE.TEST/v1"}),
            json!({"url": "https://c.example.test"}),
            json!({"url": null}),
            json!({"url": "https://a.example.test/v1", "variables": {"x": "y"}}),
            json!({"url": "https://a.example.test/v1", "other": 1}),
        ] {
            assert_eq!(
                targets(
                    &src,
                    "/ping",
                    "get",
                    Some(&configured(point, choice.clone()))
                )
                .await
                .unwrap_err()
                .code(),
                "ERR_REFUSED",
                "{version} {choice}"
            );
        }
        // Identical declarations are one server, so no choice is needed.
        let document = ping(
            version,
            Some(
                json!([{"url": "https://same.example.test"}, {"url": "https://same.example.test"}]),
            ),
        );
        assert_eq!(
            targets(&source(kind, &document, None), "/ping", "get", None)
                .await
                .unwrap(),
            "https://same.example.test/ping"
        );
        // One URL declared with different variables cannot be told apart by a choice.
        let document = ping(
            version,
            Some(json!([
                {"url": "https://{host}.example.test", "variables": {"host": {"default": "one"}}},
                {"url": "https://{host}.example.test", "variables": {"host": {"default": "two"}}}
            ])),
        );
        unsupported(
            &source(kind, &document, None),
            "server-profile-unimplemented",
        )
        .await;
    }
}

#[tokio::test]
async fn a_base_url_of_the_callers_own_is_refused_by_this_implementation() {
    for (kind, version, point, _) in KINDS {
        for servers in [
            json!([{"url": "https://only.example.test"}]),
            json!([{"url": "https://a.example.test"}, {"url": "https://b.example.test"}]),
        ] {
            let src = source(kind, &ping(version, Some(servers)), None);
            for choice in [
                json!({"baseUrl": "https://mine.example.test"}),
                json!({"baseUrl": "https://mine.example.test", "url": "https://a.example.test"}),
                json!({"baseUrl": 7}),
            ] {
                assert_eq!(
                    targets(&src, "/ping", "get", Some(&configured(point, choice)))
                        .await
                        .unwrap_err()
                        .code(),
                    "ERR_REFUSED"
                );
            }
        }
        // Not even for an implied `/` that has no `location` to resolve against.
        let src = source(kind, &ping(version, None), None);
        assert_eq!(
            targets(
                &src,
                "/ping",
                "get",
                Some(&configured(
                    point,
                    json!({"baseUrl": "https://mine.example.test"})
                ))
            )
            .await
            .unwrap_err()
            .code(),
            "ERR_REFUSED"
        );
    }
}

#[tokio::test]
async fn server_declarations_this_implementation_cannot_send_to_are_refused() {
    for (kind, version, point, _) in KINDS {
        for servers in [
            json!([{"url": "https://query.example.test/?tenant=a"}]),
            json!([{"url": "https://fragment.example.test/#top"}]),
            json!([{"url": "ftp://files.example.test"}]),
            json!([{"url": "https://{host}.example.test"}]),
            json!([{"url": "https://{host}.example.test", "variables": {"host": {}}}]),
            json!([{"url": "https://{host}.example.test", "variables": {"host": {"default": "a", "enum": ["b"]}}}]),
            json!([{"url": "https://{host.example.test", "variables": {"host": {"default": "a"}}}]),
        ] {
            let src = source(kind, &ping(version, Some(servers.clone())), Some(LOCATION));
            unsupported(&src, "server-profile-unimplemented").await;
        }
        // Beside a usable server, an unusable one is no choice.
        let document = ping(
            version,
            Some(
                json!([{"url": "https://query.example.test/?tenant=a"}, {"url": "https://plain.example.test"}]),
            ),
        );
        let src = source(kind, &document, None);
        let error = targets(&src, "/ping", "get", None).await.unwrap_err();
        assert_eq!(
            challenge(&error)["alternatives"][0]["requirements"][0]["schema"]["enum"],
            json!(["https://plain.example.test"])
        );
        assert_eq!(
            targets(
                &src,
                "/ping",
                "get",
                Some(&configured(
                    point,
                    json!({"url": "https://query.example.test/?tenant=a"})
                ))
            )
            .await
            .unwrap_err()
            .code(),
            "ERR_REFUSED"
        );
    }
}
