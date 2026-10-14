//! Gate A capability probes against fresh, independently authored expectations.
use dynamic_openapi_client::{Credential, Document, Limits, exact};
use serde_json_compat::Value;

#[test]
fn pinned_client_auth_wire_and_origin_capabilities() {
    let oracle: Value =
        serde_json_compat::from_str(include_str!("fixtures/auth/wire-oracle.json")).unwrap();
    for text in [
        include_str!("fixtures/auth/service-3.0.4.json"),
        include_str!("fixtures/auth/service-3.1.2.json"),
    ] {
        let doc = Document::parse(text.as_bytes(), None, Limits::default()).unwrap();
        for row in oracle["requests"].as_array().unwrap() {
            let op = doc
                .operations()
                .find(|o| {
                    o.path() == row["path"].as_str().unwrap()
                        && o.method().as_str() == row["method"].as_str().unwrap()
                })
                .unwrap();
            let description = op.describe().unwrap();
            let chosen = row["securitySchemes"].as_array().unwrap();
            let build = |origin: &str| {
                let selected = description.security.iter().position(|a| {
                    a.requirements.len() == chosen.len()
                        && a.requirements
                            .iter()
                            .all(|r| chosen.contains(&Value::String(r.name.clone())))
                });
                let mut input = op.request();
                if let Some(selected) = selected {
                    input = input.security(selected);
                }
                for name in chosen.iter().map(|x| x.as_str().unwrap()) {
                    let v = row["credentials"][name].as_str().unwrap();
                    let origins = vec![origin.into()];
                    let credential = if name == "ServiceToken" {
                        Credential::bearer(v, origins)
                    } else {
                        Credential::api_key(v, origins)
                    };
                    input = input.credential(name, credential);
                }
                if let Some(body) = row["bodyText"].as_str() {
                    input =
                        input.json_exact(&exact::Value::parse(body, Limits::default()).unwrap());
                }
                input.prepare()
            };
            let request = build("HTTPS://API.EXAMPLE.TEST:443").unwrap();
            for (name, value) in row["authHeaders"].as_object().unwrap() {
                let matches: Vec<_> = request
                    .headers()
                    .iter()
                    .filter(|h| h.name().eq_ignore_ascii_case(name))
                    .collect();
                assert_eq!(matches.len(), 1);
                assert_eq!(matches[0].value(), value.as_str().unwrap().as_bytes());
            }
            if !chosen.is_empty() {
                assert!(build("https://different.example.test").is_err());
            }
        }
    }
}
