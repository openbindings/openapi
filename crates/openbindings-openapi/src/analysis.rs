use crate::{Error, ErrorKind, Kind, Options, check, json};
use dynamic_openapi_client::{
    Document, Operation, ParameterLocation,
    exact::{Value, ValueKind},
    graph::{DocumentBuilder, ResourceInput},
    inspect::Method,
    schema::SchemaProfile,
    source::SourceFormat,
};
use openbindings::{JsonKind, JsonRef, JsonValue, WorkControl};
use openbindings_source_inspection::{
    BindableTarget, BindingValue, InspectionLimitation, SourceInspection,
};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

pub(crate) struct Prepared {
    pub kind: Kind,
    pub content: JsonValue,
    pub document: Document,
    pub units: Vec<Unit>,
    pub exhaustive: bool,
    pub extra_interactions: bool,
    /// Server declarations examined while preparing.
    #[cfg(test)]
    pub servers_examined: usize,
}
pub(crate) struct Unit {
    pub operation: Operation,
    pub source_ref: String,
    pub binding: JsonValue,
    pub profile: Result<Profile, &'static str>,
    /// Generated operation and binding key. `None` only for a suffixed key that
    /// still collides; see [`crate::naming::operation_keys`].
    pub key: Option<String>,
    /// Operation Object documentation, read only for a supported profile.
    pub metadata: Metadata,
}
/// Documentation carried from the Operation Object's own fields, never the Path Item's.
#[derive(Default)]
pub(crate) struct Metadata {
    description: Option<String>,
    deprecated: Option<bool>,
    tags: Option<Vec<String>>,
}
impl Metadata {
    /// A non-empty `summary` and `description` join with a blank line; either alone
    /// is used as is. `deprecated` is carried only when it is `true`. String `tags`
    /// keep their order without duplicates. Nothing else is carried.
    fn read(operation: &Value) -> Self {
        let text = |name| {
            operation
                .get(name)
                .and_then(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned))
        };
        let description = match (text("summary"), text("description")) {
            (Some(summary), Some(description)) => Some(format!("{summary}\n\n{description}")),
            (summary, description) => summary.or(description),
        };
        let deprecated = operation
            .get("deprecated")
            .is_some_and(|v| v.as_bool() == Some(true))
            .then_some(true);
        let mut tags = Vec::new();
        let listed = operation.get("tags");
        if let Some(elements) = listed.as_ref().and_then(Value::elements) {
            let mut seen = HashSet::new();
            for tag in elements {
                if let Some(tag) = tag.as_str()
                    && !seen.contains(tag)
                {
                    seen.insert(tag.to_owned());
                    tags.push(tag.to_owned());
                }
            }
        }
        Self {
            description,
            deprecated,
            tags: (!tags.is_empty()).then_some(tags),
        }
    }
    fn retained_bytes(&self) -> usize {
        self.tags.iter().flatten().fold(
            self.description.as_ref().map_or(0, String::len),
            |n, tag| n.saturating_add(tag.len()).saturating_add(32),
        )
    }
    /// Operation framing shared by inspection and synthesis.
    pub fn framing(&self, profile: &Profile) -> openbindings::Operation {
        openbindings::Operation {
            description: self.description.clone(),
            deprecated: self.deprecated,
            tags: self.tags.clone(),
            input: Some(profile.input_schema.clone()),
            ..Default::default()
        }
    }
}
pub(crate) struct Profile {
    /// The effective servers, one per distinct URL, at least one of them usable, shared
    /// with every operation whose effective list they are and charged once with it.
    pub servers: Arc<[crate::server::Server]>,
    pub parameters: BTreeMap<&'static str, BTreeMap<String, String>>,
    /// The declared request body's media, when the operation has a request body.
    pub body: Option<crate::media::RequestMedia>,
    pub input_schema: JsonValue,
    pub schema_loss: bool,
    pub security: Vec<crate::security::Alternative>,
}
impl Profile {
    /// What the operation retains of its own; its shared servers are charged with their list.
    fn retained_bytes(&self) -> usize {
        let media = self
            .body
            .as_ref()
            .map_or(0, crate::media::RequestMedia::retained_bytes);
        let security = self.security.iter().fold(media, |n, a| {
            a.keys().fold(n.saturating_add(64), |n, k| {
                n.saturating_add(k.len()).saturating_add(64)
            })
        });
        self.parameters.values().flat_map(|p| p.iter()).fold(
            security.saturating_add(self.input_schema.bytes().len()),
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
            .map(|u| {
                let mut target = BindableTarget::new(
                    u.source_ref.clone(),
                    BindingValue {
                        content: Some(u.binding.clone()),
                        ..Default::default()
                    },
                );
                target.operation_key = u.key.clone();
                target.operation = u.profile.as_ref().ok().map(|p| u.metadata.framing(p));
                target
            })
            .collect();
        if self.exhaustive && !self.extra_interactions {
            SourceInspection::complete(targets)
        } else {
            SourceInspection::partial(
                targets,
                InspectionLimitation::new(
                    "openbindings.openapi.inventory-incomplete",
                    "Supplied resources or this implementation do not establish every interaction; omitted interactions are not declared invalid",
                ),
            )
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
/// The entry document's text: a string's decoded text, or an embedded object's own
/// exact JSON text. The object is never re-serialized, so its numbers stay exact.
fn document_text(document: JsonRef<'_>) -> Result<&str, Error> {
    let invalid = || Error::new(ErrorKind::InvalidInput, "document-invalid");
    match document.kind() {
        JsonKind::String => document.as_str().ok_or_else(invalid),
        JsonKind::Object => Ok(document.text()),
        _ => Err(invalid()),
    }
}
pub(crate) fn prepare(
    kind: Kind,
    content: &JsonValue,
    options: &Options,
    control: &dyn WorkControl,
) -> Result<Prepared, Error> {
    object(content.view(), &["document", "location"])?;
    // The kinds say `location` is an absolute URI: a scheme and no fragment (RFC 3986
    // section 4.3). Anything else is invalid content, not merely unsupported.
    let location = content
        .get("location")
        .map(|v| {
            v.as_str()
                .filter(|uri| {
                    fluent_uri::UriRef::parse(*uri)
                        .is_ok_and(|u| u.scheme().is_some() && u.fragment().is_none())
                })
                .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "location-invalid"))
        })
        .transpose()?;
    let text = match content.get("document") {
        Some(document) => document_text(document)?,
        // The adapter never fetches: without `document`, nothing is analyzed.
        None if location.is_some() => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "document-retrieval-unimplemented",
            ));
        }
        None => return Err(Error::new(ErrorKind::InvalidInput, "document-required")),
    };
    // `location` is the entry document's identity and the base of its relative references
    // and relative server URLs. An absolute URI the client cannot use is unsupported.
    let input = match location {
        Some(uri) => ResourceInput::at(uri, text.as_bytes())
            .map_err(|_| Error::new(ErrorKind::Unsupported, "location-unsupported"))?,
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
    if document.format() == SourceFormat::Yaml {
        // YAML is within both kinds' domain; this implementation reads JSON only.
        return Err(Error::new(
            ErrorKind::Unsupported,
            "yaml-document-unimplemented",
        ));
    }
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
    let mut servers = crate::server::Lists::default();
    let mut seen = HashSet::new();
    let mut units = Vec::new();
    // Naming inputs per unit: lowercase method and the raw `operationId` value.
    let mut names = Vec::new();
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
                dynamic_openapi_client::graph::ReferenceKind::PathItem,
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
            match profile(
                &document,
                &operation,
                established.as_ref().ok().map(|item| item.chain.as_slice()),
                kind,
                &mut servers,
                &mut remaining,
                control,
            ) {
                Ok(profile) => Ok(profile),
                Err(NoProfile::Unsupported(reason)) => Err(reason),
                Err(NoProfile::Stopped(error)) => return Err(error),
            }
        };
        let value = operation.value();
        let metadata = if profile.is_ok() {
            Metadata::read(&value)
        } else {
            Metadata::default()
        };
        let cost = source_ref
            .len()
            .saturating_add(binding.bytes().len())
            .saturating_add(profile.as_ref().map_or(0, Profile::retained_bytes))
            .saturating_add(metadata.retained_bytes())
            .saturating_add(512);
        remaining = remaining
            .checked_sub(cost)
            .ok_or_else(|| Error::new(ErrorKind::Limit, "result-limit"))?;
        names.push((method, value.get("operationId")));
        units.push(Unit {
            operation,
            source_ref,
            binding,
            profile,
            key: None,
            metadata,
        });
    }
    check(control)?;
    let keys = {
        let candidates: Vec<_> = units
            .iter()
            .zip(&names)
            .map(|(unit, (method, id))| crate::naming::Candidate {
                method,
                path: unit.operation.path(),
                operation_id: id.as_ref().and_then(Value::as_str),
            })
            .collect();
        crate::naming::operation_keys(&candidates)
    };
    for (unit, key) in units.iter_mut().zip(keys) {
        remaining = remaining
            .checked_sub(key.as_ref().map_or(0, String::len))
            .ok_or_else(|| Error::new(ErrorKind::Limit, "result-limit"))?;
        unit.key = key;
    }
    Ok(Prepared {
        kind,
        content: content.clone(),
        document,
        units,
        exhaustive,
        extra_interactions,
        #[cfg(test)]
        servers_examined: servers.examined,
    })
}
/// Why [`profile`] has no profile for an operation.
enum NoProfile {
    /// The operation is unsupported, for this reason.
    Unsupported(&'static str),
    /// The whole source's analysis stops: its budget is spent or it was cancelled.
    Stopped(Error),
}
impl From<&'static str> for NoProfile {
    fn from(reason: &'static str) -> Self {
        Self::Unsupported(reason)
    }
}
fn profile(
    document: &Document,
    operation: &Operation,
    path_item: Option<&[Value]>,
    kind: Kind,
    servers: &mut crate::server::Lists,
    budget: &mut usize,
    control: &dyn WorkControl,
) -> Result<Profile, NoProfile> {
    if !operation.path().is_ascii() {
        return Err("literal-unicode-path-unimplemented".into());
    }
    if !matches!(
        operation.method(),
        Method::Get | Method::Post | Method::Put | Method::Patch | Method::Delete
    ) {
        return Err("unqualified-method".into());
    }
    if operation
        .value()
        .get("callbacks")
        .is_some_and(|v| v.members().is_none_or(|mut m| m.next().is_some()))
    {
        return Err("callbacks-unimplemented".into());
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
                    use dynamic_openapi_client::inspect::SecurityScheme;
                    // OpenAPI lists scope names (OAuth) or role names (other schemes)
                    // here; either kind of requirement is not implemented.
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
    let servers = servers
        .effective(document, operation, path_item, d.servers, budget, control)
        .map_err(NoProfile::Stopped)??;
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
            _ => return Err("parameter-location-unimplemented".into()),
        };
        let raw = param.resolved().target;
        let expected_style = if location == "path" { "simple" } else { "form" };
        if raw.get("content").is_some()
            || raw
                .get("style")
                .is_some_and(|v| v.as_str() != Some(expected_style))
        {
            return Err("parameter-representation-unimplemented".into());
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
            return Err("parameter-schema-dialect-unimplemented".into());
        }
        let schema = description.value;
        if kind == Kind::OpenApi30 {
            // A 3.0 Schema Reference Object's siblings do not establish a type.
            // Resolving/translating schema contracts remains outside this profile.
            if schema.get("$ref").is_some() {
                return Err("parameter-schema-reference-unimplemented".into());
            }
            // This implementation has no caller-to-wire mapping for null parameters.
            // Refuse the profile rather than synthesize a narrower claimed contract.
            if schema
                .get("nullable")
                .is_some_and(|v| v.as_bool() != Some(false))
            {
                return Err("nullable-parameter-unimplemented".into());
            }
        }
        let kind = schema
            .get("type")
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or("parameter-shape-unestablished")?;
        if !matches!(kind.as_str(), "string" | "integer" | "number" | "boolean") {
            return Err("parameter-shape-unimplemented".into());
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
    let body = d
        .request_body
        .as_ref()
        .map(|body| {
            schema_loss |= body.media.iter().any(|m| m.schema.is_some());
            crate::media::RequestMedia::declared(body)
        })
        .transpose()?;
    // The received response alone decides how it is read, so response declarations only
    // inform coverage; none makes the operation unsupported.
    for response in &d.responses {
        let declared = &response.declaration.target;
        schema_loss |= if response.status.starts_with('2') || response.status == "default" {
            // Headers never become values, and no output schema is projected.
            declared
                .get("headers")
                .is_some_and(|v| v.members().is_none_or(|mut m| m.next().is_some()))
                || response.media.iter().any(|m| m.schema.is_some())
        } else {
            // A non-2xx completes unsuccessfully with no data, so nothing it declares
            // reaches the caller.
            !response.media.is_empty() || declared.get("headers").is_some()
        };
    }
    // The native request: one member per supported parameter location, then `body`.
    let mut properties = serde_json_compat::Map::new();
    let mut required_fields = Vec::new();
    for (location, props) in locations {
        let needed = required.remove(location).unwrap_or_default();
        if !needed.is_empty() {
            required_fields.push(location);
        }
        properties.insert(location.into(), serde_json_compat::json!({"type":"object","properties":props,"required":needed,"additionalProperties":false}));
    }
    if let Some(body) = &d.request_body {
        properties.insert("body".into(), serde_json_compat::Value::Bool(true));
        if body.required {
            required_fields.push("body");
        }
    }
    let input_schema = json(&serde_json_compat::json!({"type":"object","properties":properties,"required":required_fields,"additionalProperties":false})).map_err(|_| "schema-budget")?;
    Ok(Profile {
        security,
        body,
        servers,
        parameters,
        input_schema,
        schema_loss,
    })
}
