//! Request media: which declared representation a supplied body is sent as.
//!
//! The operation's one declared media key is used. With several, the one JSON type is used
//! when there is exactly one: only concrete JSON keys count (`application/json` or a
//! `+json` type), and keys that differ only in letter case count once. A declared range
//! resolves to `application/json` only when it is the operation's one key, or when the
//! caller chooses it. Otherwise the caller names a declared key at `<kind>.requestMedia`.
//! This implementation sends JSON media only.
use dynamic_openapi_client::inspect::RequestBodyDescription;
use openbindings::JsonRef;
use openbindings_invocation::InvocationError;

/// A declared key this implementation can send, and what it is sent as.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Json<'k> {
    /// A concrete JSON media type, `application/json` or a `+json` type, sent as written.
    Concrete(&'k str),
    /// A range that matches `application/json`, which is sent for it.
    Range,
}
impl Json<'_> {
    fn sent(&self) -> &str {
        match self {
            Self::Concrete(media) => media,
            Self::Range => "application/json",
        }
    }
}

/// A declared request body's media keys and the media elected without a choice.
pub(crate) struct RequestMedia {
    /// Each declared key as written, with the JSON media sent for it, if any.
    keys: Vec<(String, Option<String>)>,
    /// The key used when the caller makes no choice.
    elected: Option<usize>,
}
impl RequestMedia {
    /// Admit a declared request body whose media this implementation can send.
    pub fn declared(body: &RequestBodyDescription) -> Result<Self, &'static str> {
        if body.media_unrestricted {
            return Err("request-media-unimplemented");
        }
        Self::from_keys(
            body.media
                .iter()
                .map(|media| (media.media_type.as_str(), media.encoding.is_some())),
        )
    }
    /// Admit the declared keys, each with whether it declares an `encoding`.
    fn from_keys<'k>(
        declared: impl ExactSizeIterator<Item = (&'k str, bool)>,
    ) -> Result<Self, &'static str> {
        let mut keys = Vec::with_capacity(declared.len());
        // The first concrete JSON key, and whether another names different media.
        let mut concrete = None;
        let mut several = false;
        for (key, encoded) in declared {
            let json = json_media(key);
            if json.is_some() && encoded {
                return Err("request-encoding-unimplemented");
            }
            if let Some(Json::Concrete(media)) = json {
                match concrete {
                    None => concrete = Some((keys.len(), media)),
                    Some((_, first)) => several |= !media.eq_ignore_ascii_case(first),
                }
            }
            keys.push((key.to_owned(), json.map(|json| json.sent().to_owned())));
        }
        if keys.iter().all(|(_, sent)| sent.is_none()) {
            return Err("request-media-unimplemented");
        }
        // A single declared key is elected, a range included; among several, only the one
        // concrete JSON type is, and a range is sent only when the caller chooses it.
        let elected = match (keys.len(), several) {
            (1, _) => Some(0),
            (_, true) => None,
            (_, false) => concrete.map(|(index, _)| index),
        };
        Ok(Self { keys, elected })
    }
    pub fn retained_bytes(&self) -> usize {
        self.keys.iter().fold(0usize, |n, (key, sent)| {
            n.saturating_add(key.len() + sent.as_ref().map_or(0, String::len) + 64)
        })
    }
    /// The media to send. `Ok(None)` means the caller must choose.
    pub fn elect(&self, choice: Option<JsonRef<'_>>) -> Result<Option<&str>, InvocationError> {
        let refused = InvocationError::refused;
        let index = match choice {
            Some(choice) => {
                let choice = choice.as_str().ok_or_else(refused)?;
                self.keys
                    .iter()
                    .position(|(key, _)| key == choice)
                    .ok_or_else(refused)?
            }
            None => match self.elected {
                Some(index) => index,
                None => return Ok(None),
            },
        };
        // A chosen key whose media is not JSON cannot be sent by this implementation.
        self.keys[index].1.as_deref().map(Some).ok_or_else(refused)
    }
    /// The declared keys a caller can choose with effect.
    pub fn choices(&self) -> impl Iterator<Item = &str> {
        self.keys
            .iter()
            .filter(|(_, sent)| sent.is_some())
            .map(|(key, _)| key.as_str())
    }
}
/// How a declared key is sent, if this implementation can send it: a concrete JSON key as
/// itself, or a range without parameters that matches `application/json` as that type.
/// Keys reach this function only after the client admitted them as media types or ranges.
fn json_media(key: &str) -> Option<Json<'_>> {
    let (essence, parameters) = key.split_once(';').unwrap_or((key, ""));
    let (kind, subtype) = essence.trim_matches([' ', '\t']).split_once('/')?;
    if kind == "*" || subtype == "*" {
        let matches = kind == "*" || kind.eq_ignore_ascii_case("application");
        return (matches && parameters.trim_matches([' ', '\t', ';']).is_empty())
            .then_some(Json::Range);
    }
    let subtype = subtype.to_ascii_lowercase();
    (kind.eq_ignore_ascii_case("application") && (subtype == "json" || subtype.ends_with("+json")))
        .then(|| Json::Concrete(key.trim_matches([' ', '\t'])))
}

#[cfg(test)]
mod tests {
    use super::{Json, RequestMedia, json_media};
    use openbindings::JsonValue;
    #[test]
    fn json_keys_send_themselves_and_matching_ranges_send_json() {
        for (key, sent) in [
            ("application/json", Some(Json::Concrete("application/json"))),
            ("Application/JSON", Some(Json::Concrete("Application/JSON"))),
            (
                "application/problem+json",
                Some(Json::Concrete("application/problem+json")),
            ),
            (
                "application/json; charset=utf-8",
                Some(Json::Concrete("application/json; charset=utf-8")),
            ),
            ("*/*", Some(Json::Range)),
            ("application/*", Some(Json::Range)),
            ("text/*", None),
            ("*/*; q=1", None),
            ("text/plain", None),
            ("application/xml", None),
            ("application/jsonx", None),
            ("multipart/form-data", None),
        ] {
            assert_eq!(json_media(key), sent, "{key}");
        }
        assert_eq!(Json::Range.sent(), "application/json");
    }
    #[test]
    fn the_one_json_type_counts_concrete_keys_only() {
        let elect = |keys: &[&str], choice: Option<&str>| {
            let media = RequestMedia::from_keys(keys.iter().map(|k| (*k, false))).unwrap();
            let choice = choice.map(|c| JsonValue::parse(format!("{c:?}")).unwrap());
            media
                .elect(choice.as_ref().map(JsonValue::view))
                .unwrap()
                .map(str::to_owned)
        };
        for (keys, sent) in [
            (
                &["application/vnd.acme+json", "*/*"][..],
                Some("application/vnd.acme+json"),
            ),
            (&["*/*", "application/json"], Some("application/json")),
            (&["application/json", "*/*"], Some("application/json")),
            (
                &["text/plain", "application/json"],
                Some("application/json"),
            ),
            // A range is sent unasked only as the one declared key.
            (&["*/*"], Some("application/json")),
            (&["application/*"], Some("application/json")),
            (&["text/plain", "*/*"], None),
            (&["application/xml", "application/*"], None),
            (&["*/*", "application/*"], None),
            // Keys that differ only in letter case count once; different JSON types do not.
            (
                &["application/json", "Application/JSON", "text/plain"],
                Some("application/json"),
            ),
            (&["application/json", "application/merge-patch+json"], None),
        ] {
            assert_eq!(elect(keys, None).as_deref(), sent, "{keys:?}");
        }
        // Chosen, a range is sent as `application/json`.
        for keys in [&["text/plain", "*/*"][..], &["*/*", "application/*"]] {
            assert_eq!(
                elect(keys, Some("*/*")).as_deref(),
                Some("application/json")
            );
        }
        assert_eq!(
            elect(&["application/xml", "application/*"], Some("application/*")).as_deref(),
            Some("application/json")
        );
        for keys in [&["text/plain"][..], &["text/*", "application/xml"]] {
            assert!(RequestMedia::from_keys(keys.iter().map(|k| (*k, false))).is_err());
        }
    }
}
