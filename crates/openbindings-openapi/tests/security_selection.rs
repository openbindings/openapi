//! Security selection: a supplied `schemes` choice wins. Otherwise, of the alternatives
//! whose every scheme has a credential in context, a single one that needs credentials is
//! used; with none, the anonymous alternative; with several, the caller chooses. When
//! nothing is satisfied and nothing is anonymous, the missing credentials are challenged.
#[path = "support/wire.rs"]
mod wire;
use openbindings_openapi::{KIND_30, KIND_31, OpenApi, SECURITY_POINT_30, SECURITY_POINT_31};
use serde_json_compat::{Value, json};
use wire::{Seen, Wire, call, challenge, host, source};

const KINDS: [(&str, &str, &str); 2] = [
    (KIND_31, "3.1.2", SECURITY_POINT_31),
    (KIND_30, "3.0.4", SECURITY_POINT_30),
];
const ORIGIN: &str = "https://secure.example.test";

fn document(version: &str) -> Value {
    let get = |security: Value| json!({"get": {"security": security, "responses": {"204": {"description": "empty"}}}});
    json!({
        "openapi": version,
        "info": {"title": "Security selection", "version": "1"},
        "servers": [{"url": ORIGIN}],
        "components": {"securitySchemes": {
            "Token": {"type": "http", "scheme": "bearer"},
            "Key": {"type": "apiKey", "in": "header", "name": "X-Key"},
            "Second": {"type": "apiKey", "in": "header", "name": "X-Second"}
        }},
        "paths": {
            "/one": get(json!([{"Token": []}])),
            "/either": get(json!([{"Token": []}, {"Key": []}])),
            "/either-or-anonymous": get(json!([{"Token": []}, {"Key": []}, {}])),
            "/anonymous-first": get(json!([{}, {"Key": []}])),
            "/pair-or-key": get(json!([{"Key": [], "Second": []}, {"Key": []}])),
            "/repeated": get(json!([{"Token": []}, {"Token": []}])),
            "/public": get(json!([]))
        }
    })
}
/// Credentials for the named schemes, each granted to the destination's origin.
fn context(point: &str, names: &[&str]) -> Value {
    let mut context = json!({"credentials": {}, "configuration": {point: {"origins": {}}}});
    for name in names {
        context["credentials"][*name] = json!(format!("fixture-{name}"));
        context["configuration"][point]["origins"][*name] = json!([ORIGIN]);
    }
    context
}
async fn send(
    src: &openbindings::Source,
    path: &str,
    context: Option<&Value>,
) -> Result<Wire, openbindings_invocation::InvocationError> {
    let seen = Seen::default();
    let provider = OpenApi::default().with_transport(host(seen.clone()));
    let result = call(&provider, src, path, "get", &["{}"], context).await;
    let seen = seen.borrow();
    match result {
        Ok(()) => Ok(seen[0].clone()),
        Err(error) => {
            assert!(seen.is_empty(), "a refusal or challenge precedes the host");
            Err(error)
        }
    }
}
/// The credential headers a request carried.
fn credentials(wire: &Wire) -> Vec<(&str, &str)> {
    ["authorization", "x-key", "x-second"]
        .into_iter()
        .filter_map(|name| wire.header(name).map(|value| (name, value)))
        .collect()
}

#[tokio::test]
async fn a_supplied_scheme_choice_wins() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        let mut both = context(point, &["Token", "Key"]);
        both["configuration"][point]["schemes"] = json!(["Key"]);
        assert_eq!(
            credentials(&send(&src, "/either", Some(&both)).await.unwrap()),
            [("x-key", "fixture-Key")]
        );
        // `[]` selects the anonymous alternative even when credentials would satisfy another.
        both["configuration"][point]["schemes"] = json!([]);
        assert!(
            credentials(
                &send(&src, "/either-or-anonymous", Some(&both))
                    .await
                    .unwrap()
            )
            .is_empty()
        );
        for schemes in [
            json!([]),
            json!(["Nope"]),
            json!(["Token", "Key"]),
            json!("Key"),
        ] {
            both["configuration"][point]["schemes"] = schemes.clone();
            assert_eq!(
                send(&src, "/either", Some(&both)).await.unwrap_err().code(),
                "ERR_REFUSED",
                "{version} {schemes}"
            );
        }
    }
}

#[tokio::test]
async fn the_one_alternative_the_credentials_satisfy_is_used() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        for (path, names, expected) in [
            (
                "/either",
                vec!["Token"],
                vec![("authorization", "Bearer fixture-Token")],
            ),
            ("/either", vec!["Key"], vec![("x-key", "fixture-Key")]),
            // A satisfied alternative that needs credentials wins over the anonymous one.
            (
                "/either-or-anonymous",
                vec!["Token"],
                vec![("authorization", "Bearer fixture-Token")],
            ),
            (
                "/anonymous-first",
                vec!["Key"],
                vec![("x-key", "fixture-Key")],
            ),
            // A credential for one scheme of a pair satisfies only the single-key alternative.
            ("/pair-or-key", vec!["Key"], vec![("x-key", "fixture-Key")]),
            // Equivalent alternatives are one choice.
            (
                "/repeated",
                vec!["Token"],
                vec![("authorization", "Bearer fixture-Token")],
            ),
        ] {
            let context = context(point, &names);
            assert_eq!(
                credentials(&send(&src, path, Some(&context)).await.unwrap()),
                expected,
                "{version} {path} {names:?}"
            );
        }
        // The interface's conveniences count as credentials for their schemes.
        for (convenience, value, expected) in [
            (
                "bearerToken",
                json!("flat-token"),
                ("authorization", "Bearer flat-token"),
            ),
            ("apiKey", json!("flat-key"), ("x-key", "flat-key")),
            (
                "apiKeys",
                json!({"Key": "named-key"}),
                ("x-key", "named-key"),
            ),
        ] {
            let mut context = context(point, &[]);
            context[convenience] = value;
            context["configuration"][point]["origins"] =
                json!({"Token": [ORIGIN], "Key": [ORIGIN]});
            assert_eq!(
                credentials(&send(&src, "/either", Some(&context)).await.unwrap()),
                [expected],
                "{version} {convenience}"
            );
        }
    }
}

#[tokio::test]
async fn without_satisfied_credentials_the_anonymous_alternative_is_used() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        for path in ["/either-or-anonymous", "/anonymous-first"] {
            for context in [None, Some(json!({})), Some(context(point, &[]))] {
                let wire = send(&src, path, context.as_ref()).await.unwrap();
                assert!(
                    credentials(&wire).is_empty(),
                    "{version} {path} {context:?}"
                );
            }
        }
        // A credential for a scheme no alternative fully uses satisfies nothing.
        let wire = send(
            &src,
            "/either-or-anonymous",
            Some(&context(point, &["Second"])),
        )
        .await
        .unwrap();
        assert!(credentials(&wire).is_empty());
    }
}

#[tokio::test]
async fn credentials_satisfying_several_alternatives_leave_the_choice_to_the_caller() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        for (path, choices) in [
            ("/either", json!([["Key"], ["Token"]])),
            ("/either-or-anonymous", json!([[], ["Key"], ["Token"]])),
            ("/pair-or-key", json!([["Key"], ["Key", "Second"]])),
        ] {
            let names = if path == "/pair-or-key" {
                vec!["Key", "Second"]
            } else {
                vec!["Token", "Key"]
            };
            let error = send(&src, path, Some(&context(point, &names)))
                .await
                .unwrap_err();
            let challenge = challenge(&error);
            let offered: Vec<Value> = challenge["alternatives"]
                .as_array()
                .unwrap()
                .iter()
                .map(|alternative| {
                    let requirements = alternative["requirements"].as_array().unwrap();
                    assert_eq!(requirements.len(), 1);
                    let requirement = &requirements[0];
                    assert_eq!(requirement["type"], "config.value");
                    assert_eq!(requirement["point"], point);
                    assert_eq!(requirement["path"], "/schemes");
                    let schema = &requirement["schema"];
                    if schema.get("const").is_some() {
                        schema["const"].clone()
                    } else {
                        schema["items"]["enum"].clone()
                    }
                })
                .collect();
            assert_eq!(Value::Array(offered), choices, "{version} {path}");
            // Choosing one resolves it.
            let mut chosen = context(point, &names);
            chosen["configuration"][point]["schemes"] =
                choices[choices.as_array().unwrap().len() - 1].clone();
            assert!(
                send(&src, path, Some(&chosen)).await.is_ok(),
                "{version} {path}"
            );
        }
    }
}

#[tokio::test]
async fn with_nothing_satisfied_or_anonymous_the_missing_credentials_are_challenged() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        // One alternative: its credential and grant.
        let error = send(&src, "/one", None).await.unwrap_err();
        assert_eq!(
            challenge(&error)["alternatives"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["requirements"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| (r["type"].clone(), r["name"].clone(), r["path"].clone()))
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [[
                (json!("auth.bearer"), json!("Token"), Value::Null),
                (json!("config.value"), Value::Null, json!("/origins/Token"))
            ]]
        );
        // Several alternatives: any one of them would do.
        for context in [None, Some(context(point, &["Second"]))] {
            let error = send(&src, "/either", context.as_ref()).await.unwrap_err();
            let challenge = challenge(&error);
            let alternatives = challenge["alternatives"].as_array().unwrap();
            assert_eq!(alternatives.len(), 2);
            let types: Vec<Vec<&str>> = alternatives
                .iter()
                .map(|a| {
                    a["requirements"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| r["type"].as_str().unwrap())
                        .collect()
                })
                .collect();
            assert_eq!(
                types,
                [
                    ["auth.apiKey", "config.value"],
                    ["auth.bearer", "config.value"]
                ]
            );
            assert!(
                challenge["target"]
                    .as_str()
                    .unwrap()
                    .contains("https://secure.example.test:443")
            );
        }
        // A grant already supplied is not asked for again.
        let mut granted = context(point, &[]);
        granted["configuration"][point]["origins"] = json!({"Key": [ORIGIN]});
        let error = send(&src, "/either", Some(&granted)).await.unwrap_err();
        let challenge = challenge(&error);
        assert_eq!(
            challenge["alternatives"][0]["requirements"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            challenge["alternatives"][0]["requirements"][0]["name"],
            "Key"
        );
        // Supplying one alternative's credentials then selects it.
        assert_eq!(
            credentials(
                &send(&src, "/either", Some(&context(point, &["Key"])))
                    .await
                    .unwrap()
            ),
            [("x-key", "fixture-Key")]
        );
    }
}

#[tokio::test]
async fn origins_and_the_earlier_refusals_stay_in_force() {
    for (kind, version, point) in KINDS {
        let src = source(kind, &document(version), None);
        // The satisfied alternative still needs a grant for this destination.
        let mut ungranted = context(point, &["Token"]);
        ungranted["configuration"][point]
            .as_object_mut()
            .unwrap()
            .remove("origins");
        let error = send(&src, "/either", Some(&ungranted)).await.unwrap_err();
        let challenge = challenge(&error);
        assert_eq!(
            challenge["alternatives"][0]["requirements"],
            json!([{
                "type": "config.value",
                "point": point,
                "path": "/origins/Token",
                "schema": {
                    "type": "array",
                    "minItems": 1,
                    "items": {"type": "string"},
                    "description": "Caller-approved credential origins; target identifies the destination",
                    "examples": [["https://secure.example.test:443"]]
                }
            }])
        );
        for (mutate, label) in [
            (
                Box::new(|c: &mut Value, point: &str| {
                    c["configuration"][point]["origins"]["Token"] =
                        json!(["https://elsewhere.test"]);
                }) as Box<dyn Fn(&mut Value, &str)>,
                "nonmatching grant",
            ),
            (
                Box::new(|c: &mut Value, _: &str| c["bearerToken"] = json!("conflicting")),
                "conflicting convenience",
            ),
            (
                Box::new(|c: &mut Value, _: &str| c["credentials"]["Token"] = json!(7)),
                "credential that is not a string",
            ),
            (
                Box::new(|c: &mut Value, point: &str| {
                    c["configuration"][point]["surprise"] = json!(true)
                }),
                "unknown security member",
            ),
            (
                Box::new(|c: &mut Value, _: &str| c["credentials"] = json!([])),
                "credentials that are not an object",
            ),
        ] {
            let mut context = context(point, &["Token"]);
            mutate(&mut context, point);
            assert_eq!(
                send(&src, "/either", Some(&context))
                    .await
                    .unwrap_err()
                    .code(),
                "ERR_REFUSED",
                "{version} {label}"
            );
        }
        // An unsecured operation ignores authentication context entirely.
        let wire = send(
            &src,
            "/public",
            Some(&json!({"credentials": [], "bearerToken": 7})),
        )
        .await
        .unwrap();
        assert!(credentials(&wire).is_empty());
    }
}
