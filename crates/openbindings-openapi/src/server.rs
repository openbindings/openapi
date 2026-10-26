//! Server choice: an operation's effective servers, their variables and relative URLs.
//!
//! The client elects the server and substitutes its variables. This module decides which
//! declaration is used and checks the caller's values before anything is prepared, and
//! derives the destination origin that credential grants are checked against.
//!
//! Server URLs come from the document and are untrusted, so their analysis is linear in
//! the declarations: each distinct list is analyzed once per prepared source and shared by
//! every operation whose effective list it is, charged to `result_bytes` as it is built,
//! and cancellable between declarations.
use crate::{Error, ErrorKind, check};
use dynamic_openapi_client::{
    Document, Operation, RequestBuilder, exact::Value, inspect::ServerDescription, uri,
};
use openbindings::{JsonKind, JsonRef, WorkControl};
use openbindings_invocation::InvocationError;
use std::{
    borrow::Cow,
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

/// One distinct effective server declaration.
pub(crate) struct Server {
    /// The URL exactly as written, which a caller names to choose it.
    pub url: String,
    /// The declaration's position in the effective list, as the client elects it.
    index: usize,
    /// The declared variables by name.
    variables: HashMap<String, Variable>,
    /// The base a relative URL resolves against: the location of the document declaring
    /// it, shared by every server of its list.
    base: Option<Arc<str>>,
    /// The resolved destination's origin, computed once when the URL has no variables.
    origin: Option<String>,
    /// Whether this implementation can send to this declaration at all.
    usable: bool,
}
struct Variable {
    default: String,
    allowed: Option<Vec<String>>,
    /// Whether the URL template substitutes it; a declared but unused variable changes nothing.
    used: bool,
}
impl Server {
    pub fn usable(&self) -> bool {
        self.usable
    }
    /// What the server retains, apart from the base its list shares.
    fn retained_bytes(&self) -> usize {
        self.variables.iter().fold(
            self.url.len() + self.origin.as_ref().map_or(0, String::len) + 64,
            |n, (name, v)| {
                v.allowed.iter().flatten().fold(
                    n.saturating_add(name.len() + v.default.len() + 64),
                    |n, a| n.saturating_add(a.len() + 32),
                )
            },
        )
    }
    fn variable(&self, name: &str) -> Option<&Variable> {
        self.variables.get(name)
    }
    /// The origin of the destination this choice resolves to, in one pass over the URL.
    pub fn origin<'a>(&'a self, choice: &Choice<'_>) -> Result<Cow<'a, str>, InvocationError> {
        if let Some(origin) = &self.origin {
            return Ok(Cow::Borrowed(origin));
        }
        let supplied = |name: &str| {
            choice
                .variables
                .and_then(|v| v.get(name))
                .and_then(JsonRef::as_str)
        };
        let url = substitute(&self.url, |name| {
            supplied(name).or_else(|| self.variable(name).map(|v| v.default.as_str()))
        })
        .ok_or_else(InvocationError::refused)?;
        target_origin(&url, self.base.as_deref())
            .map(Cow::Owned)
            .ok_or_else(InvocationError::refused)
    }
}

/// One list's servers, shared by the operations whose effective list it is, or why none
/// of them can be used.
pub(crate) type Analyzed = Result<Arc<[Server]>, &'static str>;
/// The effective server lists of one prepared source, each analyzed once and shared by
/// every operation whose effective list it is.
#[derive(Default)]
pub(crate) struct Lists {
    /// Each analyzed list by the [`identity`] of the value that identifies it.
    analyzed: HashMap<(usize, usize), Analyzed>,
    /// The entry document's own `servers` member, looked up once.
    document_servers: Option<Option<Value>>,
    /// Server declarations examined, the unit server analysis is linear in.
    #[cfg(test)]
    pub examined: usize,
}
impl Lists {
    /// The effective servers in election order, one entry per distinct URL. Two
    /// declarations of one URL with different variables cannot be told apart by a choice,
    /// so that URL is unusable. `Ok(Err(_))` means none of them can be used; `Err` stops
    /// the whole source's analysis, for its budget or for cancellation. `path_item` is
    /// the operation's resolved Path Item chain, when the analysis established it.
    pub fn effective(
        &mut self,
        document: &Document,
        operation: &Operation,
        path_item: Option<&[Value]>,
        described: Vec<ServerDescription>,
        budget: &mut usize,
        control: &dyn WorkControl,
    ) -> Result<Analyzed, Error> {
        // A declared list is identified by its first declaration; the implied `/` by the
        // empty list declaring it, or else by the entry document. That value's document is
        // also the one a relative URL resolves against.
        let declaring = match described.first().and_then(|s| s.authored.first()) {
            Some(first) => first.clone(),
            None => self.implied(document, operation, path_item),
        };
        let key = identity(&declaring);
        if let Some(analyzed) = self.analyzed.get(&key) {
            return Ok(analyzed.clone());
        }
        let analyzed = self.analyze(document, &declaring, described, budget, control)?;
        self.analyzed.insert(key, analyzed.clone());
        Ok(analyzed)
    }
    /// The value declaring an implied `/`: the effective list, which is then empty, or the
    /// entry document when none is declared. As the client elects it, the effective list is
    /// the operation's own `servers`, else the first in its Path Item chain, else the
    /// document's. Without a resolved chain, the client's inspection of the operation says.
    fn implied(
        &mut self,
        document: &Document,
        operation: &Operation,
        path_item: Option<&[Value]>,
    ) -> Value {
        let Some(chain) = path_item else {
            return operation
                .inspect()
                .servers
                .unwrap_or_else(|| document.root());
        };
        operation
            .value()
            .get("servers")
            .or_else(|| chain.iter().find_map(|item| item.get("servers")))
            .or_else(|| {
                self.document_servers
                    .get_or_insert_with(|| document.root().get("servers"))
                    .clone()
            })
            .unwrap_or_else(|| document.root())
    }
    fn analyze(
        &mut self,
        document: &Document,
        declaring: &Value,
        described: Vec<ServerDescription>,
        budget: &mut usize,
        control: &dyn WorkControl,
    ) -> Result<Analyzed, Error> {
        // The first declaration of each URL, and whether every later declaration of that
        // URL repeats its variables exactly. Later ones are dropped.
        let mut consistent: Vec<Option<bool>> = vec![None; described.len()];
        let mut first = HashMap::with_capacity(described.len());
        for (index, description) in described.iter().enumerate() {
            check(control)?;
            #[cfg(test)]
            {
                self.examined += 1;
            }
            match first.entry(description.url.as_str()) {
                Entry::Vacant(entry) => {
                    entry.insert(index);
                    consistent[index] = Some(true);
                }
                Entry::Occupied(entry) => {
                    let earlier = *entry.get();
                    let same = described[earlier].variables.as_ref().map(Value::raw)
                        == description.variables.as_ref().map(Value::raw);
                    if let Some(consistent) = &mut consistent[earlier] {
                        *consistent &= same;
                    }
                }
            }
        }
        drop(first);
        // Only a relative URL needs the base, which is looked up once for the list.
        let mut base: Option<Option<Arc<str>>> = None;
        let mut lookup = || {
            base.get_or_insert_with(|| retrieval(document, declaring).map(Arc::from))
                .clone()
        };
        let mut servers = Vec::new();
        for ((index, description), consistent) in described.into_iter().enumerate().zip(consistent)
        {
            let Some(consistent) = consistent else {
                continue;
            };
            check(control)?;
            let mut server = declare(
                index,
                description.url,
                description.variables.as_ref(),
                &mut lookup,
                control,
            )?;
            server.usable &= consistent;
            charge(budget, server.retained_bytes())?;
            servers.push(server);
        }
        if let Some(Some(base)) = &base {
            charge(budget, base.len())?;
        }
        Ok(if servers.iter().any(Server::usable) {
            Ok(Arc::from(servers))
        } else {
            Err("server-profile-unimplemented")
        })
    }
}
/// A value's identity among the prepared source's documents: the source text holding it
/// and the byte offset where the value starts there. It costs the same for any value.
fn identity(value: &Value) -> (usize, usize) {
    let offset = value.raw().as_ptr().addr() - value.source().as_ptr().addr();
    (value.source_context().id, offset)
}
fn charge(budget: &mut usize, bytes: usize) -> Result<(), Error> {
    *budget = budget
        .checked_sub(bytes)
        .ok_or_else(|| Error::new(ErrorKind::Limit, "result-limit"))?;
    Ok(())
}
fn retrieval(document: &Document, value: &Value) -> Option<String> {
    let id = value.source_context().id;
    document
        .resources()
        .find(|resource| resource.root().source_context().id == id)
        .and_then(|resource| resource.retrieval_uri().map(str::to_owned))
}
fn declare(
    index: usize,
    url: String,
    variables: Option<&Value>,
    base: &mut impl FnMut() -> Option<Arc<str>>,
    control: &dyn WorkControl,
) -> Result<Server, Error> {
    let mut server = Server {
        url,
        index,
        variables: HashMap::new(),
        base: None,
        origin: None,
        usable: false,
    };
    let Some(mut declared) = declared_variables(variables, control)? else {
        return Ok(server);
    };
    let resolved = with_defaults(&server.url, &mut declared);
    server.variables = declared;
    let Some((resolved, templated)) = resolved else {
        return Ok(server);
    };
    // Only a relative URL needs its base.
    let relative = fluent_uri::UriRef::parse(resolved.as_ref()).is_ok_and(|u| u.scheme().is_none());
    let base = if relative { base() } else { None };
    let Some(origin) = target_origin(&resolved, base.as_deref()) else {
        return Ok(server);
    };
    server.base = base;
    server.usable = true;
    if !templated {
        server.origin = Some(origin);
    }
    Ok(server)
}
/// The URL with every variable at its default, and whether it has variables. Each
/// variable the template uses is marked used; an undeclared one makes the URL unusable.
fn with_defaults<'u>(
    url: &'u str,
    declared: &mut HashMap<String, Variable>,
) -> Option<(Cow<'u, str>, bool)> {
    if !url.contains(['{', '}']) {
        return Some((Cow::Borrowed(url), false));
    }
    let pieces = pieces(url)?;
    for piece in &pieces {
        if let Piece::Variable(name) = piece {
            declared.get_mut(*name)?.used = true;
        }
    }
    let declared = &*declared;
    join(&pieces, |name| {
        declared.get(name).map(|v| v.default.as_str())
    })
    .map(|url| (Cow::Owned(url), true))
}
/// Each declared variable needs a string `default`; a declared `enum` must be a nonempty
/// list of strings that contains it. The client refuses anything else. `Ok(None)` means
/// the declarations are unusable.
fn declared_variables(
    variables: Option<&Value>,
    control: &dyn WorkControl,
) -> Result<Option<HashMap<String, Variable>>, Error> {
    let Some(variables) = variables else {
        return Ok(Some(HashMap::new()));
    };
    let Some(members) = variables.members() else {
        return Ok(None);
    };
    let mut declared = HashMap::with_capacity(members.len());
    for (name, declaration) in members {
        check(control)?;
        let Some(variable) = variable(&declaration) else {
            return Ok(None);
        };
        // The document repeats no member name, so each variable is declared once.
        declared.entry(name.to_owned()).or_insert(variable);
    }
    Ok(Some(declared))
}
fn variable(declaration: &Value) -> Option<Variable> {
    let default = declaration.get("default")?.as_str()?.to_owned();
    let allowed = match declaration.get("enum") {
        None => None,
        Some(list) => {
            let values = list
                .elements()?
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?;
            if !values.contains(&default) {
                return None;
            }
            Some(values)
        }
    };
    Some(Variable {
        default,
        allowed,
        used: false,
    })
}
enum Piece<'a> {
    Literal(&'a str),
    Variable(&'a str),
}
/// Split a URL template as the client reads it: each `{` opens a variable name that the
/// next `}` closes, and no `}` may stand outside one.
fn pieces(template: &str) -> Option<Vec<Piece<'_>>> {
    let mut pieces = vec![];
    let mut rest = template;
    while let Some(begin) = rest.find('{') {
        let end = rest[begin + 1..].find('}')? + begin + 1;
        pieces.push(Piece::Literal(&rest[..begin]));
        pieces.push(Piece::Variable(&rest[begin + 1..end]));
        rest = &rest[end + 1..];
    }
    pieces.push(Piece::Literal(rest));
    pieces
        .iter()
        .all(|p| !matches!(p, Piece::Literal(text) if text.contains('}')))
        .then_some(pieces)
}
/// Substitute each variable; a value holding a brace cannot be substituted.
fn join<'a>(pieces: &[Piece<'_>], value: impl Fn(&str) -> Option<&'a str>) -> Option<String> {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Literal(text) => out.push_str(text),
            Piece::Variable(name) => {
                let replacement = value(name)?;
                if replacement.contains(['{', '}']) {
                    return None;
                }
                out.push_str(replacement);
            }
        }
    }
    Some(out)
}
fn substitute<'a>(template: &str, value: impl Fn(&str) -> Option<&'a str>) -> Option<String> {
    join(&pieces(template)?, value)
}
/// The origin of a server URL's destination, resolving a relative URL against its base.
/// A URL with a query or fragment, or one that does not reach an `http` or `https`
/// origin, has none.
fn target_origin(url: &str, base: Option<&str>) -> Option<String> {
    let parsed = fluent_uri::UriRef::parse(url).ok()?;
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return None;
    }
    if parsed.scheme().is_some() {
        return uri::origin(url).ok();
    }
    let resolved = uri::resolve(url, base?).ok()?;
    uri::origin(&resolved).ok()
}

/// The caller's choice at `<kind>.server`, checked against the chosen declaration.
pub(crate) struct Choice<'a> {
    variables: Option<JsonRef<'a>>,
}
/// Choose one effective server. With several, an absent `url` returns `Ok(None)` so the
/// caller can challenge; every malformed or unusable choice refuses.
pub(crate) fn choose<'s, 'c>(
    servers: &'s [Server],
    point: Option<JsonRef<'c>>,
) -> Result<Option<(&'s Server, Choice<'c>)>, InvocationError> {
    let refused = InvocationError::refused;
    if let Some(point) = point {
        crate::analysis::object(point, &["url", "variables", "baseUrl"]).map_err(|_| refused())?;
        // The kinds accept a base URL of the caller's own; this implementation has no
        // client support for it yet.
        if point.get("baseUrl").is_some() {
            return Err(refused());
        }
    }
    let variables = point.and_then(|p| p.get("variables"));
    if let Some(values) = variables {
        let members = values.members().ok_or_else(refused)?;
        let mut names = std::collections::HashSet::new();
        for member in members {
            let name = member.name.as_str().ok_or_else(refused)?;
            if member.value.kind() != JsonKind::String || !names.insert(name) {
                return Err(refused());
            }
        }
    }
    let server = match point.and_then(|p| p.get("url")) {
        Some(url) => {
            let url = url.as_str().ok_or_else(refused)?;
            servers.iter().find(|s| s.url == url).ok_or_else(refused)?
        }
        None if servers.len() == 1 => &servers[0],
        None => return Ok(None),
    };
    if !server.usable {
        return Err(refused());
    }
    for member in variables
        .into_iter()
        .flat_map(|v| v.members().into_iter().flatten())
    {
        let name = member.name.as_str().ok_or_else(refused)?;
        let value = member.value.as_str().ok_or_else(refused)?;
        let declared = server.variable(name).ok_or_else(refused)?;
        if declared
            .allowed
            .as_ref()
            .is_some_and(|allowed| !allowed.iter().any(|a| a == value))
        {
            return Err(refused());
        }
    }
    Ok(Some((server, Choice { variables })))
}
/// Elect the chosen declaration and pass the supplied variables its URL substitutes.
pub(crate) fn apply(
    server: &Server,
    choice: &Choice<'_>,
    mut input: RequestBuilder,
) -> RequestBuilder {
    input = input.server(server.index);
    for member in choice
        .variables
        .into_iter()
        .flat_map(|v| v.members().into_iter().flatten())
    {
        if let (Some(name), Some(value)) = (member.name.as_str(), member.value.as_str())
            && server.variable(name).is_some_and(|v| v.used)
        {
            input = input.server_variable(name, value);
        }
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_substitute_like_the_client() {
        let value = |name: &str| match name {
            "a" => Some("x"),
            "b" => Some("y"),
            "bad" => Some("{"),
            _ => None,
        };
        assert_eq!(
            substitute("https://{a}.test/{b}", value).as_deref(),
            Some("https://x.test/y")
        );
        assert_eq!(
            substitute("https://plain.test", value).as_deref(),
            Some("https://plain.test")
        );
        for template in [
            "https://{a.test",
            "https://a}.test",
            "https://{a}}.test",
            "https://{c}.test",
            "https://{bad}.test",
        ] {
            assert!(substitute(template, value).is_none(), "{template}");
        }
    }
    #[test]
    fn origins_resolve_relative_urls_against_their_base() {
        let base = Some("https://docs.test/api/openapi.json");
        assert_eq!(
            target_origin("/v1", base).as_deref(),
            Some("https://docs.test:443")
        );
        assert_eq!(
            target_origin("v1", base).as_deref(),
            Some("https://docs.test:443")
        );
        assert_eq!(
            target_origin("//other.test:8443/v1", base).as_deref(),
            Some("https://other.test:8443")
        );
        assert_eq!(
            target_origin("http://plain.test", None).as_deref(),
            Some("http://plain.test:80")
        );
        assert!(target_origin("/v1", None).is_none());
        assert!(target_origin("https://q.test/?a=1", None).is_none());
        assert!(target_origin("https://f.test/#x", None).is_none());
        assert!(target_origin("ftp://f.test/", None).is_none());
        assert!(target_origin("/v1", Some("urn:example:doc")).is_none());
    }

    use crate::{ErrorKind, Kind, Options, analysis};
    use openbindings::JsonValue;
    use serde_json_compat::{Value as Json, json};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts how often it is asked, and reports cancellation from its `cancel_at`th ask.
    #[derive(Default)]
    struct Asked {
        asked: AtomicUsize,
        cancel_at: Option<usize>,
    }
    impl WorkControl for Asked {
        fn is_cancelled(&self) -> bool {
            let asked = self.asked.fetch_add(1, Ordering::Relaxed) + 1;
            self.cancel_at.is_some_and(|at| asked >= at)
        }
    }
    fn prepare(document: &Json, control: &Asked) -> Result<analysis::Prepared, crate::Error> {
        let content = JsonValue::parse(json!({"document": document}).to_string()).unwrap();
        analysis::prepare(Kind::OpenApi31, &content, &Options::default(), control)
    }
    fn get() -> Json {
        json!({"get": {"responses": {"204": {"description": "empty"}}}})
    }
    fn urls(prefix: &str, n: usize) -> Vec<Json> {
        (0..n)
            .map(|i| json!({"url": format!("https://{prefix}{i}.example.test")}))
            .collect()
    }
    fn servers(prepared: &analysis::Prepared, path: &str, method: &str) -> Arc<[Server]> {
        let unit = prepared
            .units
            .iter()
            .find(|u| {
                u.operation.path() == path
                    && u.operation.method().as_str().eq_ignore_ascii_case(method)
            })
            .unwrap();
        unit.profile.as_ref().ok().unwrap().servers.clone()
    }

    #[test]
    fn each_list_is_analyzed_once_and_shared_by_the_operations_inheriting_it() {
        // 30 distinct root servers, the first ten declared twice alike.
        let mut root = urls("root", 30);
        root.extend(urls("root", 10));
        let mut paths = serde_json_compat::Map::new();
        for n in 0..300 {
            paths.insert(format!("/r{n}"), get());
        }
        for item in ["a", "b"] {
            let mut declared = get();
            declared["put"] = declared["get"].clone();
            declared["servers"] = json!(urls(item, 5));
            paths.insert(format!("/{item}"), declared);
        }
        let mut own = get();
        own["get"]["servers"] = json!(urls("own", 3));
        paths.insert("/own".into(), own);
        let document = json!({
            "openapi": "3.1.2",
            "info": {"title": "Shared lists", "version": "1"},
            "servers": root,
            "paths": paths
        });
        let prepared = prepare(&document, &Asked::default()).unwrap();
        // Every declaration is examined once, however many operations inherit its list.
        assert_eq!(prepared.servers_examined, 40 + 5 + 5 + 3);
        let shared = servers(&prepared, "/r0", "get");
        assert_eq!(shared.len(), 30);
        for n in 0..300 {
            assert!(Arc::ptr_eq(
                &shared,
                &servers(&prepared, &format!("/r{n}"), "get")
            ));
        }
        assert_eq!(Arc::strong_count(&shared), 300 + 1);
        let a = servers(&prepared, "/a", "get");
        assert!(Arc::ptr_eq(&a, &servers(&prepared, "/a", "put")));
        assert!(!Arc::ptr_eq(&a, &servers(&prepared, "/b", "get")));
        assert!(!Arc::ptr_eq(&a, &shared));
        assert_eq!(servers(&prepared, "/own", "get").len(), 3);
    }

    #[test]
    fn server_analysis_checks_cancellation_between_declarations() {
        let document = json!({
            "openapi": "3.1.2",
            "info": {"title": "Cancelled", "version": "1"},
            "servers": urls("s", 2000),
            "paths": {"/x": get()}
        });
        let finished = Asked::default();
        prepare(&document, &finished).unwrap();
        // At least one check per declaration.
        let asked = finished.asked.load(Ordering::Relaxed);
        assert!(asked > 2000, "{asked}");
        // Cancelled halfway through the list, analysis stops at that check.
        let halfway = Asked {
            cancel_at: Some(asked / 2),
            ..Default::default()
        };
        let error = prepare(&document, &halfway).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::Cancelled);
        assert_eq!(halfway.asked.load(Ordering::Relaxed), asked / 2);
    }

    #[test]
    fn variables_are_looked_up_by_name_however_many_are_declared() {
        const N: usize = 20_000;
        let mut template = "https://{v0}.example.test".to_owned();
        let mut declared = serde_json_compat::Map::new();
        let mut supplied = serde_json_compat::Map::new();
        for i in 0..N {
            if i > 0 {
                template.push_str(&format!("/{{v{i}}}"));
            }
            declared.insert(format!("v{i}"), json!({"default": "d"}));
            supplied.insert(format!("v{i}"), json!(format!("s{i}")));
        }
        let variables = Value::parse(
            Json::Object(declared).to_string(),
            dynamic_openapi_client::Limits::default(),
        )
        .unwrap();
        let control = Asked::default();
        let server = declare(0, template, Some(&variables), &mut || None, &control).unwrap();
        assert!(server.usable());
        assert_eq!(server.variables.len(), N);
        assert!(server.variables.values().all(|v| v.used));
        // Unresolved until the choice is known, the origin follows the supplied value.
        assert_eq!(
            server.origin(&Choice { variables: None }).unwrap(),
            "https://d.example.test:443"
        );
        let point = JsonValue::parse(json!({"variables": supplied}).to_string()).unwrap();
        let servers = [server];
        let (chosen, choice) = choose(&servers, Some(point.view())).unwrap().unwrap();
        assert_eq!(
            chosen.origin(&choice).unwrap(),
            "https://s0.example.test:443"
        );
    }
}
