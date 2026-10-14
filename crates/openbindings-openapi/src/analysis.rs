use crate::{Error, ErrorKind, Kind, Options, check, json};
use dynamic_openapi_client::{
    Document, DocumentBuilder, Method, Operation, ParameterLocation, ResourceInput, SchemaProfile,
    SourceFormat, ValueKind,
};
use openbindings::{JsonRef, JsonValue, WorkControl};
use openbindings_source_inspection::{
    BindableTarget, BindingFragment, InspectionLimitation, SourceInspection,
};
use std::collections::{BTreeMap, HashSet};

pub(crate) struct Prepared {
    pub kind: Kind,
    pub content: JsonValue,
    pub document: Document,
    pub units: Vec<Unit>,
    pub exhaustive: bool,
    pub extra_interactions: bool,
}
pub(crate) struct Unit {
    pub operation: Operation,
    pub source_ref: String,
    pub binding: JsonValue,
    pub profile: Result<Profile, &'static str>,
}
pub(crate) struct Profile {
    pub servers: Vec<String>,
    pub parameters: BTreeMap<&'static str, BTreeMap<String, String>>,
    pub has_body: bool,
    pub input_schema: JsonValue,
    pub schema_loss: bool,
    pub security: Vec<crate::security::Alternative>,
    pub empty_responses: EmptyResponses,
}
/// One bit per successful status, compiled once from the effective declarations.
/// This carries no credential or request data across transport execution.
#[derive(Clone, Copy, Default)]
pub(crate) struct EmptyResponses(u128);
impl EmptyResponses {
    fn from_descriptions(responses: &[dynamic_openapi_client::ResponseDescription]) -> Self {
        let fallback = ["2XX", "default"]
            .into_iter()
            .find_map(|key| responses.iter().find(|r| r.status == key));
        let mut bits = if fallback.is_some_and(|r| r.media.is_empty()) {
            (1u128 << 100) - 1
        } else {
            0
        };
        for response in responses {
            if let Ok(status @ 200..=299) = response.status.parse::<u16>() {
                let bit = 1u128 << (status - 200);
                if response.media.is_empty() {
                    bits |= bit;
                } else {
                    bits &= !bit;
                }
            }
        }
        Self(bits)
    }
    pub fn contains(self, status: u16) -> bool {
        (200..300).contains(&status) && self.0 & (1u128 << (status - 200)) != 0
    }
}
impl Profile {
    fn retained_bytes(&self) -> usize {
        let servers = self
            .servers
            .iter()
            .fold(0usize, |n, s| n.saturating_add(s.len()).saturating_add(32));
        let servers = self.security.iter().fold(servers, |n, a| {
            a.keys().fold(n.saturating_add(64), |n, k| {
                n.saturating_add(k.len()).saturating_add(64)
            })
        });
        self.parameters.values().flat_map(|p| p.iter()).fold(
            servers
                .saturating_add(self.input_schema.bytes().len())
                .saturating_add(std::mem::size_of::<EmptyResponses>()),
            |n, (name, kind)| {
                n.saturating_add(name.len())
                    .saturating_add(kind.len())
                    .saturating_add(128)
            },
        )
    }
}
impl Prepared {
    pub fn inspection(&self) -> SourceInspection {
        let targets = self
            .units
            .iter()
            .map(|u| BindableTarget {
                source_ref: u.source_ref.clone(),
                binding: BindingFragment {
                    content: Some(u.binding.clone()),
                    ..Default::default()
                },
                operation_key: None,
                operation: u.profile.as_ref().ok().map(|p| openbindings::Operation {
                    input: Some(p.input_schema.clone()),
                    ..Default::default()
                }),
            })
            .collect();
        if self.exhaustive && !self.extra_interactions {
            SourceInspection::complete(targets)
        } else {
            SourceInspection::partial(targets, InspectionLimitation { code: "openbindings.openapi.inventory-incomplete".into(), message: "Supplied resources or this implementation do not establish every interaction; omitted interactions are not declared invalid".into(), details: None, additional_fields: Default::default() })
        }
    }
}
pub(crate) fn object(value: JsonRef<'_>, allowed: &[&str]) -> Result<(), Error> {
    let members = value
        .members()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "expected-object"))?;
    let mut names = HashSet::new();
    for member in members {
        let name = member
            .name
            .as_str()
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid-member-name"))?;
        if !allowed.contains(&name) || !names.insert(name) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "unknown-or-duplicate-member",
            ));
        }
    }
    Ok(())
}
pub(crate) fn prepare(
    kind: Kind,
    content: &JsonValue,
    options: &Options,
    control: &dyn WorkControl,
) -> Result<Prepared, Error> {
    object(content.view(), &["document", "retrievalUri"])?;
    let text = content
        .get("document")
        .and_then(JsonRef::as_str)
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "document-text-required"))?;
    let input = match content.get("retrievalUri") {
        Some(v) => ResourceInput::at(
            v.as_str()
                .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "retrieval-uri-required"))?,
            text.as_bytes(),
        )
        .map_err(|_| Error::new(ErrorKind::Unsupported, "retrieval-uri-unsupported"))?,
        None => ResourceInput::anonymous(text.as_bytes()),
    };
    let mut builder = DocumentBuilder::new(options.limits.clone(), options.graph_limits.clone());
    let entry = builder
        .add(input)
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "document-admission"))?;
    for resource in &options.resources {
        check(control)?;
        builder
            .add(resource.clone())
            .map_err(|_| Error::new(ErrorKind::Unreadable, "resource-admission"))?;
    }
    let document = builder
        .build(entry)
        .map_err(|_| Error::new(ErrorKind::Unreadable, "graph-admission"))?;
    if document.format() != SourceFormat::Json || document.edition().ok() != Some(kind.edition()) {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "unqualified-description",
        ));
    }
    let root = document.root();
    let mut exhaustive = document.discovery_complete()
        && root
            .get("paths")
            .is_none_or(|p| p.kind() == ValueKind::Object);
    let mut extra_interactions = root
        .get("webhooks")
        .is_some_and(|v| v.members().is_none_or(|mut m| m.next().is_some()));
    let mut remaining = options.result_bytes;
    let mut seen = HashSet::new();
    let mut units = Vec::new();
    for operation in document.operations() {
        check(control)?;
        let method = operation.method().as_str().to_ascii_lowercase();
        if !matches!(
            method.as_str(),
            "get" | "post" | "put" | "patch" | "delete" | "head" | "options" | "trace"
        ) || !operation.path().starts_with('/')
        {
            exhaustive = false;
            continue;
        }
        let item_pointer = format!(
            "/paths/{}",
            operation.path().replace('~', "~0").replace('/', "~1")
        );
        let established = document
            .protocol_at(
                &item_pointer,
                dynamic_openapi_client::ReferenceKind::PathItem,
            )
            .and_then(|site| document.resolve_protocol(&site));
        if let Ok(item) = &established
            && !item.chain.iter().any(|v| {
                v.get(&method)
                    .is_some_and(|v| v.kind() == ValueKind::Object)
            })
        {
            exhaustive = false;
            continue;
        }
        let ambiguous = established.as_ref().is_ok_and(|item| {
            [&*method, "parameters", "servers"]
                .iter()
                .any(|key| item.chain.iter().filter(|v| v.get(key).is_some()).count() > 1)
        });
        if !seen.insert((operation.path().to_owned(), method.clone())) {
            exhaustive = false;
            continue;
        }
        let source_ref = format!(
            "/paths/{}/{}",
            operation.path().replace('~', "~0").replace('/', "~1"),
            method
        );
        let binding = json(&serde_json_compat::json!({"path":operation.path(),"method":method}))?;
        extra_interactions |= operation
            .value()
            .get("callbacks")
            .is_some_and(|v| v.members().is_none_or(|mut m| m.next().is_some()));
        let profile = if ambiguous {
            Err("ambiguous-path-item-merge")
        } else {
            profile(&operation, kind)
        };
        let cost = source_ref
            .len()
            .saturating_add(binding.bytes().len())
            .saturating_add(profile.as_ref().map_or(0, Profile::retained_bytes))
            .saturating_add(512);
        remaining = remaining
            .checked_sub(cost)
            .ok_or_else(|| Error::new(ErrorKind::Limit, "result-limit"))?;
        units.push(Unit {
            operation,
            source_ref,
            binding,
            profile,
        });
    }
    check(control)?;
    Ok(Prepared {
        kind,
        content: content.clone(),
        document,
        units,
        exhaustive,
        extra_interactions,
    })
}
fn profile(operation: &Operation, kind: Kind) -> Result<Profile, &'static str> {
    if !operation.path().is_ascii() {
        return Err("literal-unicode-path-unimplemented");
    }
    if !matches!(
        operation.method(),
        Method::Get | Method::Post | Method::Put | Method::Patch | Method::Delete
    ) {
        return Err("unqualified-method");
    }
    if operation
        .value()
        .get("callbacks")
        .is_some_and(|v| v.members().is_none_or(|mut m| m.next().is_some()))
    {
        return Err("callbacks-unimplemented");
    }
    let d = operation
        .describe()
        .map_err(|_| "operation-description-unavailable")?;
    let security = d
        .security
        .iter()
        .map(|a| {
            let mut headers = HashSet::new();
            a.requirements
                .iter()
                .map(|r| {
                    use crate::security::Family;
                    use dynamic_openapi_client::SecurityScheme;
                    if !r.scopes.is_empty() {
                        return Err("security-roles-unimplemented");
                    }
                    let wire_name = match &r.scheme {
                        SecurityScheme::Bearer { .. } => Some("authorization"),
                        SecurityScheme::ApiKey {
                            name,
                            location: ParameterLocation::Header,
                        } => Some(name.as_str()),
                        _ => None,
                    };
                    if let Some(name) = wire_name
                        && (name.is_empty()
                            || !name.bytes().all(|b| {
                                b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
                            })
                            || (d.request_body.is_some()
                                && name.eq_ignore_ascii_case("content-type")))
                    {
                        return Err("security-header-unusable");
                    }
                    if let Some(name) = wire_name
                        && !headers.insert(name.to_ascii_lowercase())
                    {
                        return Err("security-header-collision-unimplemented");
                    }
                    let family = match &r.scheme {
                        SecurityScheme::Bearer { .. } => Family::Bearer,
                        SecurityScheme::ApiKey {
                            location: ParameterLocation::Header,
                            ..
                        } => Family::Key,
                        _ => return Err("security-unimplemented"),
                    };
                    Ok((r.name.clone(), family))
                })
                .collect::<Result<crate::security::Alternative, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    if d.servers.is_empty()
        || d.servers.iter().any(|s| {
            s.variables
                .as_ref()
                .is_some_and(|v| v.members().is_none_or(|mut m| m.next().is_some()))
                || dynamic_openapi_client::uri::origin(&s.url).is_err()
                || s.url.contains('?')
        })
    {
        return Err("server-profile-unimplemented");
    }
    let mut locations =
        BTreeMap::<&str, serde_json_compat::Map<String, serde_json_compat::Value>>::from([
            ("path", Default::default()),
            ("query", Default::default()),
        ]);
    let mut required = BTreeMap::<&str, Vec<String>>::new();
    let mut schema_loss = false;
    let mut parameters: BTreeMap<&'static str, BTreeMap<String, String>> = BTreeMap::new();
    for param in d.parameters.iter() {
        let location = match param.location() {
            ParameterLocation::Path => "path",
            ParameterLocation::Query => "query",
            _ => return Err("parameter-location-unimplemented"),
        };
        let raw = param.resolved().target;
        let expected_style = if location == "path" { "simple" } else { "form" };
        if raw.get("content").is_some()
            || raw
                .get("style")
                .is_some_and(|v| v.as_str() != Some(expected_style))
        {
            return Err("parameter-representation-unimplemented");
        }
        let description = param
            .schema_description()
            .ok_or("parameter-shape-unestablished")?;
        if kind == Kind::OpenApi31
            && !matches!(&description.profile, SchemaProfile::JsonSchema { dialect_uri }
                if matches!(dialect_uri.as_str(),
                    "https://spec.openapis.org/oas/3.1/dialect/base"
                    | "https://spec.openapis.org/oas/3.1/dialect/2024-11-10"
                    | "https://json-schema.org/draft/2020-12/schema"))
        {
            // Use the client's lexical context, including schema-level overrides.
            // Other dialects may ignore or reinterpret the apparent type keyword.
            return Err("parameter-schema-dialect-unimplemented");
        }
        let schema = description.value;
        if kind == Kind::OpenApi30 {
            // A 3.0 Schema Reference Object's siblings do not establish a type.
            // Resolving/translating schema contracts remains outside this profile.
            if schema.get("$ref").is_some() {
                return Err("parameter-schema-reference-unimplemented");
            }
            // This experiment has no caller-to-wire mapping for null parameters.
            // Refuse the profile rather than synthesize a narrower claimed contract.
            if schema
                .get("nullable")
                .is_some_and(|v| v.as_bool() != Some(false))
            {
                return Err("nullable-parameter-unimplemented");
            }
        }
        let kind = schema
            .get("type")
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or("parameter-shape-unestablished")?;
        if !matches!(kind.as_str(), "string" | "integer" | "number" | "boolean") {
            return Err("parameter-shape-unimplemented");
        }
        parameters
            .entry(location)
            .or_default()
            .insert(param.name().into(), kind.clone());
        locations
            .entry(location)
            .or_default()
            .insert(param.name().into(), serde_json_compat::json!({"type":kind}));
        if param.required() {
            required
                .entry(location)
                .or_default()
                .push(param.name().into());
        }
        schema_loss |= schema
            .members()
            .is_none_or(|mut m| m.any(|(k, _)| k != "type"));
    }
    if let Some(body) = &d.request_body {
        if body.media_unrestricted
            || body.media.len() != 1
            || body.media[0].media_type != "application/json"
        {
            return Err("request-media-unimplemented");
        }
        if body.media[0].encoding.is_some() {
            return Err("request-encoding-unimplemented");
        }
        schema_loss |= body.media[0].schema.is_some();
    }
    for response in &d.responses {
        if !response.status.starts_with('2') && response.status != "default" {
            schema_loss |=
                !response.media.is_empty() || response.declaration.target.get("headers").is_some();
            continue;
        }
        if response
            .media
            .iter()
            .any(|m| m.media_type != "application/json")
        {
            return Err("response-media-unimplemented");
        }
        if response
            .declaration
            .target
            .get("headers")
            .is_some_and(|v| v.members().is_none_or(|mut m| m.next().is_some()))
        {
            return Err("response-headers-unimplemented");
        }
        schema_loss |= response.media.iter().any(|m| m.schema.is_some());
    }
    let mut parameter_properties = serde_json_compat::Map::new();
    let mut required_locations = Vec::new();
    for (location, props) in locations {
        let needed = required.remove(location).unwrap_or_default();
        if !needed.is_empty() {
            required_locations.push(location);
        }
        parameter_properties.insert(location.into(), serde_json_compat::json!({"type":"object","properties":props,"required":needed,"additionalProperties":false}));
    }
    let mut properties = serde_json_compat::Map::new();
    let mut required_fields = Vec::new();
    if !parameter_properties.is_empty() {
        if !required_locations.is_empty() {
            required_fields.push("parameters");
        }
        properties.insert("parameters".into(), serde_json_compat::json!({"type":"object","properties":parameter_properties,"required":required_locations,"additionalProperties":false}));
    }
    if let Some(body) = &d.request_body {
        properties.insert("body".into(), serde_json_compat::Value::Bool(true));
        if body.required {
            required_fields.push("body");
        }
    }
    let input_schema = json(&serde_json_compat::json!({"type":"object","properties":properties,"required":required_fields,"additionalProperties":false})).map_err(|_| "schema-budget")?;
    Ok(Profile {
        empty_responses: EmptyResponses::from_descriptions(&d.responses),
        security,
        has_body: d.request_body.is_some(),
        servers: d.servers.into_iter().map(|s| s.url).collect(),
        parameters,
        input_schema,
        schema_loss,
    })
}
