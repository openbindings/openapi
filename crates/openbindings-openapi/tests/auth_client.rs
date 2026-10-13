//! Gate A capability probes against fresh, independently authored expectations.
use dynamic_openapi_client::{
    Body, Credential, CredentialValue, Document, ExactJson, Input, Limits,
};
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
            let mut input = Input::default();
            input.selection.security = description.security.iter().position(|a| {
                a.requirements.len() == chosen.len()
                    && a.requirements
                        .iter()
                        .all(|r| chosen.contains(&Value::String(r.name.clone())))
            });
            for name in chosen.iter().map(|x| x.as_str().unwrap()) {
                let v = row["credentials"][name].as_str().unwrap().to_owned();
                input.credentials.insert(
                    name.into(),
                    Credential {
                        value: if name == "ServiceToken" {
                            CredentialValue::Bearer(v)
                        } else {
                            CredentialValue::ApiKey(v)
                        },
                        origins: vec!["HTTPS://API.EXAMPLE.TEST:443".into()],
                    },
                );
            }
            if let Some(body) = row["bodyText"].as_str() {
                input.body = Body::Json(ExactJson::parse(body, Limits::default()).unwrap().root());
            }
            let request = op.prepare(&input).unwrap();
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
                for c in input.credentials.values_mut() {
                    c.origins = vec!["https://different.example.test".into()];
                }
                assert!(op.prepare(&input).is_err());
            }
        }
    }
}
