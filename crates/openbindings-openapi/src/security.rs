//! Security policy for the private kinds. Source interpretation and HTTP encoding stay in the client.
use crate::{Kind, Options, analysis::Profile};
use dynamic_openapi_client::{Credential, RequestBuilder};
use openbindings::{JsonRef, JsonValue, WorkControl};
use openbindings_invocation::{ContextChallenge, InvocationError};
use serde_json_compat::{Value, json as j};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Bearer,
    Key,
}
impl Family {
    fn requirement(self) -> &'static str {
        match self {
            Self::Bearer => "auth.bearer",
            Self::Key => "auth.apiKey",
        }
    }
    /// The interface's flat convenience for this family.
    fn flat_name(self) -> &'static str {
        match self {
            Self::Bearer => "bearerToken",
            Self::Key => "apiKey",
        }
    }
}
pub(crate) type Alternative = BTreeMap<String, Family>;

pub(crate) fn details(profile: &Profile) -> Value {
    j!({"alternatives":profile.security.iter().map(|a|a.iter().map(|(n,f)|j!({"type":f.requirement(),"name":n})).collect::<Vec<_>>()).collect::<Vec<_>>()})
}

/// The operation and destination credentials are selected for. Unsecured operations read
/// no credentials.
pub(crate) struct Target<'a> {
    pub kind: Kind,
    pub path: &'a str,
    pub method: &'a str,
    /// The origin of the chosen destination, which credential grants must name.
    pub origin: &'a str,
}

/// The credential material a context supplies, as the interface defines it: named
/// `credentials`, and the `apiKeys`, `bearerToken` and `apiKey` conveniences.
struct Supplied<'a> {
    root: Option<JsonRef<'a>>,
    credentials: Option<JsonRef<'a>>,
    keys: Option<JsonRef<'a>>,
}
impl Supplied<'_> {
    /// Whether the context has a credential for one scheme of an alternative. A flat
    /// convenience counts only for an alternative with exactly one scheme of its family.
    fn has(&self, alternative: &Alternative, name: &str, family: Family) -> bool {
        self.credentials.and_then(|c| c.get(name)).is_some()
            || (family == Family::Key && self.keys.and_then(|k| k.get(name)).is_some())
            || (self.root.and_then(|r| r.get(family.flat_name())).is_some()
                && alternative.values().filter(|f| **f == family).count() == 1)
    }
    fn satisfies(&self, alternative: &Alternative) -> bool {
        alternative
            .iter()
            .all(|(name, family)| self.has(alternative, name, *family))
    }
}

pub(crate) fn apply(
    profile: &Profile,
    context: Option<&JsonValue>,
    target: Target<'_>,
    options: &Options,
    work: &dyn WorkControl,
    mut input: RequestBuilder,
) -> Result<RequestBuilder, InvocationError> {
    let fail = InvocationError::refused;
    let kind = target.kind;
    if !profile.security.iter().any(|a| !a.is_empty()) {
        if !profile.security.is_empty() {
            input = input.security(0);
        }
        return Ok(input);
    }
    // The invocation admitted the whole context before reading it; credentials need it
    // to be an object as well.
    if context.is_some_and(|c| c.view().members().is_none()) {
        return Err(fail());
    }
    let root = context.map(JsonValue::view);
    let configuration = member_object(root, "configuration")?;
    let point = member_object(configuration, kind.security_point())?;
    if let Some(p) = point {
        crate::analysis::object(p, &["schemes", "origins"]).map_err(|_| fail())?;
    }
    let origin = target.origin;
    let supplied = Supplied {
        root,
        credentials: member_object(root, "credentials")?,
        keys: member_object(root, "apiKeys")?,
    };
    let origins = member_object(point, "origins")?;
    // Only challenges need the sorted, deduplicated public choice inventory.
    // Successful attempts compare borrowed source keys without rebuilding it.
    let choice_inventory = std::cell::OnceCell::new();
    let choices = || {
        choice_inventory.get_or_init(|| {
            profile
                .security
                .iter()
                .enumerate()
                .rev()
                .map(|(i, a)| (a.keys().map(String::as_str).collect(), i))
                .collect::<BTreeMap<Vec<&str>, usize>>()
        })
    };
    let challenge = || {
        let mut identity = Bounded::new(options.result_bytes);
        identity.value(&j!([
            kind.label(),
            target.path,
            target.method,
            origin,
            choices().keys().collect::<Vec<_>>()
        ]))?;
        Challenge::new(&identity.text()?, options.result_bytes)
    };
    // A grant releases a credential only to caller-approved origins. Every supplied grant
    // is validated; one that does not name the destination refuses rather than prompting.
    let grant = |name: &str| {
        origins
            .and_then(|o| o.get(name))
            .map(|v| {
                let mut result = vec![];
                let mut matches_target = false;
                for v in v.elements().ok_or_else(fail)? {
                    if work.is_cancelled() {
                        return Err(InvocationError::cancelled());
                    }
                    let raw = v.as_str().ok_or_else(fail)?;
                    // Validate all supplied grants; the client performs the final target check.
                    matches_target |=
                        dynamic_openapi_client::uri::origin(raw).map_err(|_| fail())? == origin;
                    result.push(raw.to_owned());
                }
                if !matches_target {
                    return Err(fail());
                }
                Ok(result)
            })
            .transpose()
    };
    let selected = if let Some(s) = point.and_then(|p| p.get("schemes")) {
        // A supplied choice wins: an exact set of scheme names, `[]` for anonymous.
        let mut names = BTreeSet::new();
        for v in s.elements().ok_or_else(fail)? {
            if work.is_cancelled() {
                return Err(InvocationError::cancelled());
            }
            if !names.insert(v.as_str().ok_or_else(fail)?) {
                return Err(fail());
            }
        }
        profile
            .security
            .iter()
            .position(|a| {
                a.len() == names.len() && a.keys().map(String::as_str).eq(names.iter().copied())
            })
            .ok_or_else(fail)?
    } else {
        // Otherwise the alternatives whose every scheme has a credential decide: exactly
        // one that needs credentials is used; with none, the anonymous alternative.
        let mut satisfied: Option<usize> = None;
        let mut several = false;
        for (i, a) in profile.security.iter().enumerate() {
            if work.is_cancelled() {
                return Err(InvocationError::cancelled());
            }
            if a.is_empty() || !supplied.satisfies(a) {
                continue;
            }
            match satisfied {
                None => satisfied = Some(i),
                Some(first) if !profile.security[first].keys().eq(a.keys()) => {
                    several = true;
                    break;
                }
                Some(_) => {}
            }
        }
        if several {
            // The caller's own credentials leave the choice open: the caller chooses.
            let mut challenge = challenge()?;
            for names in choices().keys() {
                if work.is_cancelled() {
                    return Err(InvocationError::cancelled());
                }
                challenge.begin_alternative()?;
                challenge.selector(kind.security_point(), names)?;
                challenge.end_alternative()?;
            }
            return Err(challenge.finish()?);
        }
        match satisfied.or_else(|| profile.security.iter().position(Alternative::is_empty)) {
            Some(index) => index,
            None => {
                // Nothing is satisfied and nothing is anonymous: ask for the missing
                // credentials of each alternative, any one of which would do.
                let mut challenge = challenge()?;
                for &index in choices().values() {
                    let alternative = &profile.security[index];
                    challenge.begin_alternative()?;
                    for (name, family) in alternative {
                        if work.is_cancelled() {
                            return Err(InvocationError::cancelled());
                        }
                        if !supplied.has(alternative, name, *family) {
                            challenge.auth(family.requirement(), name)?;
                        }
                        if grant(name)?.is_none() {
                            challenge.grant(kind.security_point(), name, origin)?;
                        }
                    }
                    challenge.end_alternative()?;
                }
                return Err(challenge.finish()?);
            }
        }
    };
    input = input.security(selected);
    let selected = &profile.security[selected];
    if selected.is_empty() {
        return Ok(input);
    }
    let mut missing = None;
    let bearer_count = selected.values().filter(|f| **f == Family::Bearer).count();
    let key_count = selected.len() - bearer_count;
    for (name, family) in selected {
        if work.is_cancelled() {
            return Err(InvocationError::cancelled());
        }
        let flat = root.and_then(|r| r.get(family.flat_name()));
        if flat.is_some()
            && (if *family == Family::Bearer {
                bearer_count
            } else {
                key_count
            }) != 1
        {
            return Err(fail());
        }
        let material = consistent([
            supplied.credentials.and_then(|c| c.get(name)),
            if *family == Family::Key {
                supplied.keys.and_then(|k| k.get(name))
            } else {
                None
            },
            flat,
        ])?;
        let grant = grant(name)?;
        if material.is_none() || grant.is_none() {
            if missing.is_none() {
                let mut c = challenge()?;
                c.begin_alternative()?;
                missing = Some(c);
            }
            let c = missing.as_mut().expect("initialized");
            if material.is_none() {
                c.auth(family.requirement(), name)?;
            }
            if grant.is_none() {
                c.grant(kind.security_point(), name, origin)?;
            }
        }
        if let (Some(value), Some(origins)) = (material, grant) {
            let value = match family {
                Family::Bearer => Credential::bearer(value, origins),
                Family::Key => Credential::api_key(value, origins),
            };
            input = input.credential(name, value);
        }
    }
    if let Some(mut challenge) = missing {
        challenge.end_alternative()?;
        return Err(challenge.finish()?);
    }
    Ok(input)
}
fn member_object<'a>(
    parent: Option<JsonRef<'a>>,
    name: &str,
) -> Result<Option<JsonRef<'a>>, InvocationError> {
    let v = parent.and_then(|p| p.get(name));
    if v.is_some_and(|v| v.members().is_none()) {
        return Err(InvocationError::refused());
    }
    Ok(v)
}
fn consistent<'a>(values: [Option<JsonRef<'a>>; 3]) -> Result<Option<&'a str>, InvocationError> {
    let mut found = None;
    for v in values.into_iter().flatten() {
        let v = v.as_str().ok_or_else(InvocationError::refused)?;
        if found.is_some_and(|old| old != v) {
            return Err(InvocationError::refused());
        }
        found = Some(v);
    }
    Ok(found)
}
// Serialize incrementally through a bounded writer. No repeated-origin Value tree
// is built first, and no truncated challenge is returned as a usable alternative.
struct Bounded {
    bytes: Vec<u8>,
    limit: usize,
}
impl std::io::Write for Bounded {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("challenge limit"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Bounded {
    fn new(limit: usize) -> Self {
        Self {
            bytes: vec![],
            limit,
        }
    }
    fn raw(&mut self, s: &str) -> Result<(), InvocationError> {
        std::io::Write::write_all(self, s.as_bytes()).map_err(|_| InvocationError::refused())
    }
    fn string(&mut self, s: &str) -> Result<(), InvocationError> {
        serde_json_compat::to_writer(self, s).map_err(|_| InvocationError::refused())
    }
    fn value(&mut self, v: &Value) -> Result<(), InvocationError> {
        serde_json_compat::to_writer(self, v).map_err(|_| InvocationError::refused())
    }
    fn text(self) -> Result<String, InvocationError> {
        String::from_utf8(self.bytes).map_err(|_| InvocationError::refused())
    }
}
struct Challenge {
    writer: Bounded,
    alternatives: usize,
    requirements: usize,
}
impl Challenge {
    fn new(target: &str, limit: usize) -> Result<Self, InvocationError> {
        let mut writer = Bounded::new(limit);
        writer.raw("{\"target\":")?;
        writer.string(target)?;
        writer.raw(",\"alternatives\":[")?;
        Ok(Self {
            writer,
            alternatives: 0,
            requirements: 0,
        })
    }
    fn begin_alternative(&mut self) -> Result<(), InvocationError> {
        if self.alternatives > 0 {
            self.writer.raw(",")?;
        }
        self.alternatives += 1;
        self.requirements = 0;
        self.writer.raw("{\"requirements\":[")
    }
    fn begin_requirement(&mut self, family: &str) -> Result<(), InvocationError> {
        if self.requirements > 0 {
            self.writer.raw(",")?;
        }
        self.requirements += 1;
        self.writer.raw("{\"type\":")?;
        self.writer.string(family)
    }
    fn auth(&mut self, family: &str, name: &str) -> Result<(), InvocationError> {
        self.begin_requirement(family)?;
        self.writer.raw(",\"name\":")?;
        self.writer.string(name)?;
        self.writer.raw("}")
    }
    fn config(&mut self, point: &str, path: &str) -> Result<(), InvocationError> {
        self.begin_requirement("config.value")?;
        self.writer.raw(",\"point\":")?;
        self.writer.string(point)?;
        self.writer.raw(",\"path\":")?;
        self.writer.string(path)?;
        self.writer.raw(",\"schema\":")
    }
    fn selector(&mut self, point: &str, names: &[&str]) -> Result<(), InvocationError> {
        self.config(point, "/schemes")?;
        if names.is_empty() {
            self.writer.raw("{\"const\":[]}")?;
        } else {
            self.writer
                .raw("{\"type\":\"array\",\"uniqueItems\":true,\"minItems\":")?;
            self.writer.raw(&names.len().to_string())?;
            self.writer.raw(",\"maxItems\":")?;
            self.writer.raw(&names.len().to_string())?;
            self.writer.raw(",\"items\":{\"enum\":[")?;
            for (i, n) in names.iter().enumerate() {
                if i > 0 {
                    self.writer.raw(",")?;
                }
                self.writer.string(n)?;
            }
            self.writer.raw("]}}")?;
        }
        self.writer.raw("}")
    }
    fn grant(&mut self, point: &str, name: &str, origin: &str) -> Result<(), InvocationError> {
        self.config(
            point,
            &format!("/origins/{}", name.replace('~', "~0").replace('/', "~1")),
        )?;
        self.writer.raw("{\"type\":\"array\",\"minItems\":1,\"items\":{\"type\":\"string\"},\"description\":\"Caller-approved credential origins; target identifies the destination\",\"examples\":[[")?;
        self.writer.string(origin)?;
        self.writer.raw("]]}}")
    }
    fn end_alternative(&mut self) -> Result<(), InvocationError> {
        self.writer.raw("]}")
    }
    fn finish(mut self) -> Result<InvocationError, InvocationError> {
        self.writer.raw("]}")?;
        let v = JsonValue::parse(self.writer.text()?).map_err(|_| InvocationError::refused())?;
        ContextChallenge::new(v)
            .map(InvocationError::challenge)
            .map_err(|_| InvocationError::refused())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn challenge(limit: usize) -> Result<InvocationError, InvocationError> {
        let mut c = Challenge::new("target", limit)?;
        c.begin_alternative()?;
        c.auth("auth.bearer", "Token")?;
        c.end_alternative()?;
        c.finish()
    }
    #[test]
    fn exact_challenge_budget_and_one_over() {
        let e = challenge(4096).unwrap();
        let full = e.to_json().unwrap();
        let n = full.get("data").expect("challenge data").text().len();
        assert!(challenge(n).is_ok());
        assert_eq!(challenge(n - 1).unwrap_err().code(), "ERR_REFUSED");
    }
    #[test]
    fn repeated_origins_stop_at_the_incremental_limit() {
        let origin = format!("https://{}.test", "a".repeat(32000));
        let mut c = Challenge::new("target", 65536).unwrap();
        c.begin_alternative().unwrap();
        c.grant("point", "One", &origin).unwrap();
        c.grant("point", "Two", &origin).unwrap();
        assert!(c.grant("point", "Three", &origin).is_err());
        assert!(c.writer.bytes.len() <= 65536);
    }
}
