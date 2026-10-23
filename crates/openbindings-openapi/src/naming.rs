//! Generated operation keys for one enumerated source.
//!
//! Keys are computed over every enumerated operation, including those this
//! implementation cannot represent, so a change in what it supports never moves
//! another operation's key. No key depends on enumeration order.
use std::collections::{HashMap, hash_map::Entry};

/// One enumerated operation's naming inputs.
pub(crate) struct Candidate<'a> {
    /// Lowercase method field name.
    pub method: &'a str,
    /// Exact Paths Object key.
    pub path: &'a str,
    /// The Operation Object's `operationId`, when it is a string.
    pub operation_id: Option<&'a str>,
}

/// One key per candidate, in candidate order.
///
/// A unique non-empty `operationId` supplies the key, made to fit the core key
/// grammar; otherwise the method and path derive it. Operations that would share
/// a key each take their derived key with a path hash suffix: 8 hexadecimal
/// digits, then all 16 for a suffixed key that still collides. A suffixed key
/// that collides even then is `None`; unsuffixed keys are already unique.
pub(crate) fn operation_keys(candidates: &[Candidate<'_>]) -> Vec<Option<String>> {
    let duplicated = shared(candidates.iter().map(|c| usable(c.operation_id)));
    let mut keys: Vec<_> = candidates
        .iter()
        .zip(&duplicated)
        .map(|(c, duplicated)| match usable(c.operation_id) {
            Some(id) if !duplicated => sanitized(id),
            _ => derived_key(c.method, c.path),
        })
        .collect();
    let colliding = shared(keys.iter().map(|key| Some(key.as_str())));
    if !colliding.contains(&true) {
        return keys.into_iter().map(Some).collect();
    }
    let suffixed = |c: &Candidate<'_>, digits: usize| {
        let hash = format!(
            "{:016x}",
            fnv1a64(format!("{} {}", c.method, c.path).as_bytes())
        );
        format!("{}_{}", derived_key(c.method, c.path), &hash[..digits])
    };
    for index in (0..keys.len()).filter(|&i| colliding[i]) {
        keys[index] = suffixed(&candidates[index], 8);
    }
    let still = shared(keys.iter().map(|key| Some(key.as_str())));
    for index in (0..keys.len()).filter(|&i| colliding[i] && still[i]) {
        keys[index] = suffixed(&candidates[index], 16);
    }
    let last = shared(keys.iter().map(|key| Some(key.as_str())));
    keys.into_iter()
        .enumerate()
        .map(|(i, key)| (!(colliding[i] && last[i])).then_some(key))
        .collect()
}

/// An `operationId` is usable when it is a non-empty string.
fn usable(id: Option<&str>) -> Option<&str> {
    id.filter(|id| !id.is_empty())
}

/// The core key grammar's later characters: `[A-Za-z0-9_.-]`.
fn key_character(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// Replace characters outside the key grammar with `_`, and prefix `_` when the
/// result starts with `.` or `-`. A key already matching the grammar is unchanged.
/// Case is never converted.
fn sanitized(id: &str) -> String {
    if id.chars().all(key_character) && !id.starts_with(['.', '-']) {
        return id.to_owned();
    }
    let mut key = String::with_capacity(id.len() + 1);
    if id.starts_with(['.', '-']) {
        key.push('_');
    }
    key.extend(id.chars().map(|c| if key_character(c) { c } else { '_' }));
    key
}

/// The lowercase method, then each non-empty path segment's contribution,
/// joined by `_`. A contribution drops template braces, replaces characters
/// outside the key grammar with `_`, collapses runs of `_` and trims them from
/// both ends; a contribution left empty is omitted.
fn derived_key(method: &str, path: &str) -> String {
    let mut key = String::with_capacity(method.len() + path.len());
    key.push_str(method);
    let mut part = String::new();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        part.clear();
        for c in segment.chars().filter(|c| !matches!(c, '{' | '}')) {
            let c = if key_character(c) { c } else { '_' };
            if !(c == '_' && part.ends_with('_')) {
                part.push(c);
            }
        }
        let contribution = part.trim_matches('_');
        if !contribution.is_empty() {
            key.push('_');
            key.push_str(contribution);
        }
    }
    key
}

/// Whether each present item equals any other item (exact comparison). Each
/// item is hashed once; absent items are never shared.
fn shared<'a>(items: impl ExactSizeIterator<Item = Option<&'a str>>) -> Vec<bool> {
    let mut first = HashMap::with_capacity(items.len());
    let mut shared = vec![false; items.len()];
    for (index, item) in items.enumerate() {
        let Some(item) = item else { continue };
        match first.entry(item) {
            Entry::Occupied(entry) => {
                shared[*entry.get()] = true;
                shared[index] = true;
            }
            Entry::Vacant(entry) => {
                entry.insert(index);
            }
        }
    }
    shared
}

/// FNV-1a, 64-bit: offset basis 0xcbf29ce484222325, prime 0x100000001b3.
/// Each byte is XORed into the hash, which is then multiplied by the prime.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_published_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn derived_keys_omit_contributions_left_empty() {
        assert_eq!(derived_key("get", "/"), "get");
        assert_eq!(derived_key("get", "/{}/x/%%"), "get_x");
        assert_eq!(derived_key("put", "/a__b//_c_/"), "put_a_b_c");
    }
}
