//! Reading a received response. The received response decides: the description's
//! response declarations never change how a response is read.
//!
//! A final 2xx succeeds only with complete evidence: dispatched with no execution error
//! and no cancellation, a complete body, and header delivery `Complete` with nothing
//! omitted. A 204, a response to HEAD and any other 2xx whose content is zero octets emit
//! no value; content on a 205 fails. Otherwise the `Content-Type` decides the one value:
//! JSON media become the JSON value, text a string, and anything else, including no
//! `Content-Type`, a Base64 string of the octets. Nothing from a header reaches the value.
use dynamic_openapi_client::{
    Header, Limits, Outcome,
    transport::{BodyState, DispatchEvidence, HeaderDelivery, Provenance},
};
use openbindings::{JsonKind, JsonLimits, JsonMember, JsonRef, JsonValue};
use openbindings_invocation::InvocationError;
use std::{borrow::Cow, collections::HashSet};

const BYTE_ORDER_MARK: &[u8] = b"\xEF\xBB\xBF";

/// The value a received response emits, `Ok(None)` for success without a value.
/// `head` says whether the request was a HEAD request.
pub(crate) fn read(
    outcome: &Outcome,
    head: bool,
    limits: &Limits,
) -> Result<Option<JsonValue>, InvocationError> {
    let failed = InvocationError::execution_failed;
    if outcome.cancelled() {
        return Err(InvocationError::cancelled());
    }
    // A host outcome is evidence, not permission to infer absence of effects.
    if outcome.error().is_some() || outcome.dispatch() != DispatchEvidence::Dispatched {
        return Err(failed());
    }
    let status = outcome.status().ok_or_else(failed)?;
    if outcome.body_state() != Some(BodyState::Complete)
        || outcome.header_delivery() != Some(HeaderDelivery::Complete)
        || outcome.omitted_header_lines() != Some(0)
    {
        return Err(failed());
    }
    // A protocol outcome reaches the operation only through the binding's adaptation,
    // and none is supported yet: a final non-2xx completes unsuccessfully with no data.
    if !(200..300).contains(&status) {
        return Err(failed());
    }
    let headers = outcome.headers().ok_or_else(failed)?;
    let content = outcome.raw().ok_or_else(failed)?;
    // HTTP framing ends a 204 and a response to HEAD at the header section, so they carry
    // no content; delivered bytes there are evidence HTTP cannot produce.
    if status == 204 || head {
        return if content.is_empty() {
            Ok(None)
        } else {
            Err(failed())
        };
    }
    // This implementation removes no content coding; a host that removed them says so.
    if !matches!(
        outcome.provenance(),
        Some(Provenance::Decoded | Provenance::HostDelivered)
    ) && coded(headers)
    {
        return Err(failed());
    }
    if content.is_empty() {
        return Ok(None);
    }
    // A server must not generate content on a 205, and HTTP framing does not prevent it.
    if status == 205 {
        return Err(failed());
    }
    value(headers, content, limits).map(Some).ok_or_else(failed)
}

/// Whether `Content-Encoding` lists a coding other than `identity`. Its field lines form
/// one comma-separated list of codings in the order applied (RFC 9110 section 8.4).
fn coded(headers: &[Header]) -> bool {
    headers
        .iter()
        .filter(|h| h.name().eq_ignore_ascii_case("content-encoding"))
        .flat_map(|h| h.value().split(|&b| b == b','))
        .map(ows_trim)
        .any(|coding| !coding.is_empty() && !coding.eq_ignore_ascii_case(b"identity"))
}

/// The value of non-empty content, chosen by its one `Content-Type`; `None` when the
/// response cannot become a value.
fn value(headers: &[Header], content: &[u8], limits: &Limits) -> Option<JsonValue> {
    let mut fields = headers
        .iter()
        .filter(|h| h.name().eq_ignore_ascii_case("content-type"));
    // No `Content-Type` is read as `application/octet-stream`, without sniffing.
    let Some(field) = fields.next() else {
        return base64(content);
    };
    if fields.next().is_some() {
        return None;
    }
    let media = MediaType::parse(field.value())?;
    if media.is_json() {
        json(content, limits)
    } else if media.is_text() {
        JsonValue::string(text(&media, content)?).ok()
    } else {
        base64(content)
    }
}

/// One media type, as `Content-Type` carries it (RFC 9110 section 8.3.1).
struct MediaType<'a> {
    kind: &'a [u8],
    subtype: &'a [u8],
    /// The last `charset` parameter's value, unquoted, and how many there are.
    charset: Option<Cow<'a, [u8]>>,
    charsets: usize,
}
impl<'a> MediaType<'a> {
    /// `type "/" subtype *( OWS ";" OWS [ parameter ] )`, or `None` when the field value
    /// does not parse as one media type. A list of media types does not parse.
    fn parse(value: &'a [u8]) -> Option<Self> {
        let mut at = 0;
        let kind = token(value, &mut at)?;
        if value.get(at) != Some(&b'/') {
            return None;
        }
        at += 1;
        let subtype = token(value, &mut at)?;
        let (mut charset, mut charsets) = (None, 0);
        loop {
            ows(value, &mut at);
            match value.get(at) {
                None => break,
                Some(b';') => at += 1,
                Some(_) => return None,
            }
            ows(value, &mut at);
            if matches!(value.get(at), None | Some(b';')) {
                continue;
            }
            let name = token(value, &mut at)?;
            if value.get(at) != Some(&b'=') {
                return None;
            }
            at += 1;
            let parameter = if value.get(at) == Some(&b'"') {
                quoted(value, &mut at)?
            } else {
                Cow::Borrowed(token(value, &mut at)?)
            };
            if name.eq_ignore_ascii_case(b"charset") {
                charsets += 1;
                charset = Some(parameter);
            }
        }
        Some(Self {
            kind,
            subtype,
            charset,
            charsets,
        })
    }
    /// `application/json`, or any type with the `+json` structured syntax suffix.
    fn is_json(&self) -> bool {
        (self.kind.eq_ignore_ascii_case(b"application")
            && self.subtype.eq_ignore_ascii_case(b"json"))
            || (self.subtype.len() >= 5
                && self.subtype[self.subtype.len() - 5..].eq_ignore_ascii_case(b"+json"))
    }
    /// `text/*`, or any other type with a `charset` parameter.
    fn is_text(&self) -> bool {
        self.kind.eq_ignore_ascii_case(b"text") || self.charsets > 0
    }
}

/// The string text content holds in its charset, or `None` when it is not valid there.
/// Text without a `charset`, or with `utf-8`, is read as UTF-8, and a leading byte order
/// mark is not part of the string. `us-ascii` text must hold octets below 0x80 only.
/// Every other charset, and a repeated `charset` parameter, is not read: this is the one
/// place a decision to decode more charsets changes.
fn text<'a>(media: &MediaType<'_>, content: &'a [u8]) -> Option<&'a str> {
    let utf8 =
        || std::str::from_utf8(content.strip_prefix(BYTE_ORDER_MARK).unwrap_or(content)).ok();
    match media.charset.as_deref() {
        _ if media.charsets > 1 => None,
        None => utf8(),
        Some(charset) if charset.eq_ignore_ascii_case(b"utf-8") => utf8(),
        Some(charset) if charset.eq_ignore_ascii_case(b"us-ascii") => content
            .is_ascii()
            .then(|| std::str::from_utf8(content).ok())
            .flatten(),
        Some(_) => None,
    }
}

/// The JSON value of content in JSON media, parsed once from the delivered bytes with
/// exact numbers. A leading byte order mark is ignored. Duplicate member names take the
/// last occurrence's value, at every depth; only a value that has duplicates is rebuilt.
/// Invalid JSON, including an escape denoting an unpaired surrogate, is `None`.
fn json(content: &[u8], limits: &Limits) -> Option<JsonValue> {
    let content = content.strip_prefix(BYTE_ORDER_MARK).unwrap_or(content);
    let sdk = JsonLimits::default();
    // The client's response bounds, within the SDK's own: a member name and its value
    // count as up to two parsed nodes where the client counts one value.
    let limits = JsonLimits {
        max_bytes: limits.body_bytes.min(sdk.max_bytes),
        max_depth: limits.depth.min(sdk.max_depth),
        max_nodes: limits.nodes.saturating_mul(2).min(sdk.max_nodes),
    };
    let value = JsonValue::parse_with_limits(content, limits).ok()?;
    if unpaired_surrogate_escape(content) {
        return None;
    }
    if value.has_duplicate_names() {
        last_occurrences(&value, limits)
    } else {
        Some(value)
    }
}

/// Whether valid JSON text holds a `\u` escape denoting an unpaired UTF-16 surrogate: a
/// high surrogate not followed by a low surrogate escape, or a low surrogate alone.
fn unpaired_surrogate_escape(text: &[u8]) -> bool {
    let unit = |at: usize| -> Option<u16> {
        let digits = text.get(at..at + 4)?;
        digits.iter().try_fold(0u16, |unit, &digit| {
            Some((unit << 4) | u16::try_from((digit as char).to_digit(16)?).ok()?)
        })
    };
    let mut at = 0;
    while let Some(found) = text[at..].iter().position(|&b| b == b'\\') {
        let escape = at + found;
        if text.get(escape + 1) != Some(&b'u') {
            // A two-character escape, `\\` included.
            at = escape + 2;
            continue;
        }
        at = escape + 6;
        match unit(escape + 2) {
            Some(0xD800..=0xDBFF) => {
                let low = text.get(at..at + 2) == Some(b"\\u")
                    && unit(at + 2).is_some_and(|u| (0xDC00..=0xDFFF).contains(&u));
                if !low {
                    return true;
                }
                at += 6;
            }
            Some(0xDC00..=0xDFFF) | None => return true,
            Some(_) => {}
        }
    }
    false
}

/// Rebuild a value with duplicate member names, keeping each name's last occurrence in
/// its own position, at every depth. Containers without duplicates and every scalar
/// keep their exact text, numbers included.
fn last_occurrences(value: &JsonValue, limits: JsonLimits) -> Option<JsonValue> {
    enum Step<'a> {
        Value(JsonRef<'a>),
        Text(&'a [u8]),
    }
    let mut out = Vec::with_capacity(value.bytes().len());
    let mut steps = vec![Step::Value(value.view())];
    while let Some(step) = steps.pop() {
        let value = match step {
            Step::Text(text) => {
                out.extend_from_slice(text);
                continue;
            }
            Step::Value(value) => value,
        };
        let rebuilt = matches!(value.kind(), JsonKind::Array | JsonKind::Object)
            && value.to_owned().has_duplicate_names();
        if !rebuilt {
            out.extend_from_slice(value.text().as_bytes());
            continue;
        }
        // Steps run last in, first out: push each container's closing text, then its
        // parts from last to first.
        if let Some(elements) = value.elements() {
            out.push(b'[');
            steps.push(Step::Text(b"]"));
            let elements: Vec<_> = elements.collect();
            for (index, element) in elements.into_iter().enumerate().rev() {
                steps.push(Step::Value(element));
                if index > 0 {
                    steps.push(Step::Text(b","));
                }
            }
        } else {
            out.push(b'{');
            steps.push(Step::Text(b"}"));
            let members: Vec<JsonMember<'_>> = value.members()?.collect();
            let mut seen = HashSet::with_capacity(members.len());
            // From the last member back: each name's first sighting is its last occurrence.
            let mut kept = Vec::with_capacity(members.len());
            for member in members.iter().rev() {
                if seen.insert(member.name.as_str()?) {
                    kept.push(*member);
                }
            }
            let count = kept.len();
            for (index, member) in kept.into_iter().enumerate() {
                steps.push(Step::Value(member.value));
                steps.push(Step::Text(b":"));
                steps.push(Step::Text(member.name.text().as_bytes()));
                if index + 1 < count {
                    steps.push(Step::Text(b","));
                }
            }
        }
    }
    JsonValue::parse_owned_with_limits(out, limits).ok()
}

/// The octets as a JSON string of their padded Base64 encoding (RFC 4648 section 4).
fn base64(content: &[u8]) -> Option<JsonValue> {
    let limits = JsonLimits::default();
    let length = content.len().div_ceil(3).checked_mul(4)?.checked_add(2)?;
    if length > limits.max_bytes {
        return None;
    }
    let mut out = Vec::with_capacity(length);
    out.push(b'"');
    base64_into(content, &mut out);
    out.push(b'"');
    JsonValue::parse_owned_with_limits(out, limits).ok()
}
fn base64_into(content: &[u8], out: &mut Vec<u8>) {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in content.chunks(3) {
        let octet = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let group = (octet(0) << 16) | (octet(1) << 8) | octet(2);
        let sextet = |shift: u32| ALPHABET[((group >> shift) & 0x3F) as usize];
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { b'=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { b'=' });
    }
}

/// RFC 9110 `tchar`.
fn tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}
fn token<'a>(value: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    let start = *at;
    while value.get(*at).copied().is_some_and(tchar) {
        *at += 1;
    }
    (*at > start).then(|| &value[start..*at])
}
fn ows(value: &[u8], at: &mut usize) {
    while matches!(value.get(*at), Some(b' ' | b'\t')) {
        *at += 1;
    }
}
fn ows_trim(value: &[u8]) -> &[u8] {
    let start = value.iter().position(|b| !matches!(b, b' ' | b'\t'));
    let end = value.iter().rposition(|b| !matches!(b, b' ' | b'\t'));
    match (start, end) {
        (Some(start), Some(end)) => &value[start..=end],
        _ => &[],
    }
}
/// An RFC 9110 `quoted-string` starting at `at`, with its quoted pairs unescaped.
fn quoted<'a>(value: &'a [u8], at: &mut usize) -> Option<Cow<'a, [u8]>> {
    let start = *at + 1;
    let mut unescaped: Option<Vec<u8>> = None;
    let mut i = start;
    loop {
        let b = *value.get(i)?;
        match b {
            b'"' => break,
            b'\\' => {
                let escaped = *value.get(i + 1)?;
                if !(escaped == b'\t' || escaped == b' ' || (escaped >= 0x21 && escaped != 0x7F)) {
                    return None;
                }
                unescaped
                    .get_or_insert_with(|| value[start..i].to_vec())
                    .push(escaped);
                i += 2;
            }
            b'\t' | b' ' | 0x21 | 0x23..=0x5B | 0x5D..=0x7E | 0x80..=0xFF => {
                if let Some(unescaped) = &mut unescaped {
                    unescaped.push(b);
                }
                i += 1;
            }
            _ => return None,
        }
    }
    *at = i + 1;
    Some(match unescaped {
        Some(unescaped) => Cow::Owned(unescaped),
        None => Cow::Borrowed(&value[start..i]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_4648_test_vectors() {
        for (input, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            let mut out = Vec::new();
            base64_into(input.as_bytes(), &mut out);
            assert_eq!(out, encoded.as_bytes(), "{input}");
            assert_eq!(
                base64(input.as_bytes()).unwrap().text(),
                format!("\"{encoded}\"")
            );
        }
        let mut out = Vec::new();
        base64_into(&[0x00, 0xFF, 0xFE, 0x3E, 0x3F], &mut out);
        assert_eq!(out, b"AP/+Pj8=");
    }

    #[test]
    fn media_types_parse_as_one_type_with_parameters() {
        for (value, parsed) in [
            ("application/json", Some(("application", "json", None, 0))),
            (
                "Text/Plain;Charset=UTF-8",
                Some(("Text", "Plain", Some("UTF-8"), 1)),
            ),
            (
                "text/plain ; charset=\"utf-8\"",
                Some(("text", "plain", Some("utf-8"), 1)),
            ),
            (
                "text/plain; charset=\"u\\tf-8\"",
                Some(("text", "plain", Some("utf-8"), 1)),
            ),
            ("text/plain;", Some(("text", "plain", None, 0))),
            ("text/plain; ;a=b", Some(("text", "plain", None, 0))),
            (
                "text/plain; charset=a; charset=b",
                Some(("text", "plain", Some("b"), 2)),
            ),
            ("text/plain, application/json", None),
            ("text/plain; charset", None),
            ("text/plain; charset=\"open", None),
            ("text /plain", None),
            ("text/", None),
            ("/plain", None),
            ("text", None),
            ("", None),
            ("text/plain; a=b c", None),
        ] {
            let media = MediaType::parse(value.as_bytes());
            assert_eq!(
                media.map(|m| (
                    String::from_utf8(m.kind.to_vec()).unwrap(),
                    String::from_utf8(m.subtype.to_vec()).unwrap(),
                    m.charset
                        .map(|c| String::from_utf8(c.into_owned()).unwrap()),
                    m.charsets
                )),
                parsed.map(|(k, s, c, n): (&str, &str, Option<&str>, usize)| (
                    k.to_owned(),
                    s.to_owned(),
                    c.map(str::to_owned),
                    n
                )),
                "{value}"
            );
        }
    }

    // This implementation refuses HEAD operations before any request, so a response to
    // HEAD is read here directly.
    #[tokio::test]
    async fn a_response_to_head_carries_no_content() {
        use dynamic_openapi_client::{
            Cancellation, Document,
            transport::{HostCapabilities, ResponseInput, TransportResult, UploadState},
        };
        let document = Document::parse(
            r#"{"openapi":"3.1.2","info":{"title":"Head","version":"1"},"servers":[{"url":"https://head.example.test"}],"paths":{"/x":{"head":{"responses":{"200":{"description":"ok","content":{"application/json":{}}}}}}}}"#.as_bytes(),
            None,
            Limits::default(),
        )
        .unwrap();
        let request = document
            .operations()
            .next()
            .unwrap()
            .request()
            .prepare()
            .unwrap();
        for (head, body, coding, expected) in [
            (true, "", None, Ok(None)),
            (true, "", Some("gzip"), Ok(None)),
            (true, "[1]", None, Err(())),
            (false, "[1]", None, Ok(Some("[1]"))),
            (false, "", None, Ok(None)),
            (false, "", Some("gzip"), Err(())),
        ] {
            let outcome = request
                .invoke(
                    Cancellation::default(),
                    &HostCapabilities::programmable(),
                    |_, _| async move {
                        let mut headers = vec![
                            Header::new("content-type", "application/json"),
                            Header::new("content-length", "3"),
                        ];
                        headers.extend(coding.map(|c| Header::new("content-encoding", c)));
                        let mut response = ResponseInput::new(
                            200,
                            headers,
                            body.as_bytes().into(),
                            BodyState::Complete,
                            Provenance::Wire,
                        );
                        response.header_delivery = HeaderDelivery::Complete;
                        TransportResult {
                            response: Some(response),
                            upload: UploadState::Complete,
                            error_detail: None,
                        }
                    },
                )
                .await;
            let read = read(&outcome, head, &Limits::default());
            assert_eq!(
                read.as_ref()
                    .map(|v| v.as_ref().map(|v| v.text()))
                    .map_err(|e| e.code()),
                expected.map_err(|()| "ERR_EXECUTION_FAILED"),
                "head={head} body={body} coding={coding:?}"
            );
        }
    }

    #[test]
    fn surrogate_escapes_must_pair() {
        for (text, unpaired) in [
            (r#""plain""#, false),
            (r#""😀""#, false),
            (r#""😀""#, false),
            (r#""\\ud83d""#, false),
            (r#""é\n""#, false),
            (r#""\ud83d""#, true),
            (r#""\ude00""#, true),
            (r#""\ud83dA""#, true),
            (r#""\ud83dx""#, true),
            (r#"{"\udfff":1}"#, true),
            (r#""\\\ud83d""#, true),
        ] {
            assert_eq!(
                unpaired_surrogate_escape(text.as_bytes()),
                unpaired,
                "{text}"
            );
        }
    }
}
