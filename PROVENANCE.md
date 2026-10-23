# Clean import provenance

Imported from SDK commit `78724ae`, full revision
`78724aedca8d0dc104aa3be6254b4a225b7e9b58` in
[openbindings/sdk](https://github.com/openbindings/sdk).
This repository receives one clean source import, with no filtered history.
Historical adapter evidence and development records are archived outside both
product repositories. At import, all adapter `src/` and test `.rs` files and all fixture bytes were identical.
The stopped measurement examples remain sources, compiled but never executed by CI.

The root development manifest, CI workflow, shared qualification helper and
dependency attribution files are new extraction plumbing. The consumer test binary
serializes its own tests with a local lock because they count process-global owners.
`NOTICE` describes this resolved graph. No SDK implementation is copied into this
product repository. The current immutable Git pins are SDK
`3546fc29d3fb329656c6a7506f6006b9eb38375d` and client
`5cc2cf47ad9a05d1aa3cf968d03841ca9bbded61`.

Earlier migrations adopted validation vocabulary, companion-owned cancellation,
caller-owned prepared operations, and the client's checked request builders,
scoped credentials and advanced transport execution. The adapter retains its
bounded source-analysis cache and the existing finite OpenAPI profile.

The current SDK migration requires conformance proof before operation invocation or
normative value-contract preparation. Consumers validate reloaded documents
and pass the resulting `Conformant<Document>` directly; attaching an evaluator is
infallible, and a conformant operation's `input()` and `output()` hand out its value
contracts for preparation. Reads on a plain `Document` stay
fallible; reads on a conformant document use the SDK's infallible views, whose text
is `&str` because the applied specification excludes unpaired-surrogate escapes from
conformant documents. No local compatibility API
restores the unproven-document contract route.

`PreparedValueContract::validate` is the cheap check: a failure carries the fixed message
`value does not satisfy the contract`, no problems, and `problems_complete = false`.
`explain` returns the same verdict with the library's first error and still does not
claim complete diagnostics; its allocation and stack use follow the library's bounds.
Each problem names the failing instance and schema keyword by exact pointer; the
holder of the value, document or supplied resource computes positions on demand.
The operation invoker withholds a rejected value on its verdict alone. The adapter
supplies a binding provider and builds no operation invoker; its consumers keep that
default because they act on the invocation error code and read no problems. An
application that reports why a value was rejected opts in with
`with_validation_detail(Detail::Explain)`.
Document structural/schema findings use the fixed message
`value violates the fixed normative schema` at the narrowest established failing
source position; consumers must use evidence and codes rather than old wording
or counts. These are SDK diagnostic changes, not changes to the adapter's source
coverage or wire profile.

Reference inspection now uses the SDK's separate `ReferenceRefusal` and
`ReferenceRefusalReason`; bounds planning is private and ordinary schema reference
resolution follows the library. The adapter implements none of those internals.
Its source references still use the independent OpenAPI client's graph. Synthesized
input contracts contain envelope constraints and scalar types, without `$ref` or
source schema projection. Consequently static-reference hole evaluation does not
change their verdicts or refusals: malformed envelopes fail and accepted envelopes
satisfy. Author-added positive reference contracts may now decide independently of
a known static defect, or return `ValueOutcome::Undefined { detail }` when proven
completions disagree. This replaces `NoVerdictReason::Undefined`; missing resources
and implementation limits remain distinct no-verdict refusals. The SDK operation
invoker maps undefined and no-verdict outcomes to `ERR_SCHEMA_UNRESOLVED` and
withholds the value. The adapter delegates this check to that invoker and has no
exhaustive value-outcome match of its own. Its existing satisfies/fails assertions
and synthesized envelope contracts keep their results.

The final SDK pin also requires callers to consume `DocumentValidation` results.
Public SDK debug formatting redacts caller and document text; this does not change
wire values or adapter decisions.
SDK package inventory, crate documentation and TypeScript-only API changes require
no adapter behavior changes. The adapter keeps its existing Rust 1.99 toolchain,
which exceeds the SDK's Rust 1.88 minimum.

The companion API uses `CoverageClaim::complete` for initial complete coverage and
`ProviderInput` as a stream without boxing a future for each input read. Input cardinality, closure, cancellation and conditional `Send` are preserved.
Provider records may gain fields in later SDK versions, so the adapter builds them with
their constructors: `SupportedKind::new` with `with_description`, `BindableTarget::new`,
`InspectionLimitation::new`, `CoverageLimitation::new` with `with_details`, and
`SynthesisCandidate::new`. An optional operation framing is assigned to its public
field. Kind descriptions, limitation evidence and candidates are unchanged.

The SDK review's API changes ask little of the adapter. Edit pointers are now a
`PointerRef`, borrowed from escaped text or an exact `Pointer`, so synthesis and the
JSON-path consumer's authentication fixture convert their `Edit::Set` text pointers
with `into()`; edited bytes are unchanged. A prepared operation now keeps a share of its
invoker rather than borrowing it, so the measurement example holds its operation
invoker in an `Arc` before `prepare`. Advisory preflight is a provided
`BindingInvoker::preflight` hook that declares no job by default, replacing the
separate `BindingPreflight` trait and the `preflight_with_provider` facade methods.
The adapter keeps that default: it had no preflight of its own, and its server and
security challenges stay where invocation raises them, after input admission, so a
facade preflight still admits the request and predicts nothing. `PreparedValueContract`
is the prepared form's new name. Equality on SDK records and outcomes, `#[must_use]`
on builder methods, the value-scaled default evaluation budget, the 32-group default
pattern nesting and the separate `evaluation-depth-limit` code change no adapter
result: synthesized envelope contracts contain no patterns, and every existing
assertion keeps its expected value.

An earlier pin's evaluator fixes changed no adapter code. Its `unevaluatedProperties`
walk built only the `patternProperties` patterns, fixing a validation panic; no
adapter contract or fixture uses either keyword. Its preparation charged schema
compilation by the work it did and refused beyond its allowance with `LimitExceeded`
and `preparation-work-limit`, which synthesized envelope contracts stayed far inside.
Its resource registration was linear, and its documented `$dynamicRef` limitation
needed a dynamic anchor, which no adapter contract declares.

The pin after it composed the default evaluator over the unmodified registry
`jsonschema` 0.58.6 library. One adapter test changed its expected evidence, not its
result. An author contract whose `$ref` names a plain-name anchor its resource does
not declare (`#missing`) used to prepare and give `ValueOutcome::Undefined` with
`static-reference-defect` at the `$ref` keyword. From that pin on, the default evaluator
never returns `Undefined` and refuses such a contract at preparation, with
`NoVerdictReason::ConservativePreparation`, code `anchor-missing` and the location of
the schema that holds the reference. The SDK operation invoker still withholds the
value with `ERR_SCHEMA_UNRESOLVED`, now at `ValidationStage::Preparation`. Plain
invocation prepares on the first value, so a refused input contract still never
reaches the provider and a refused output contract still follows one provider call.
The test asserts that whole refusal record, and its name no longer says undefined. The
preparation work charge, its `preparation-work-limit` code and `regex_steps` are gone
with the backtracking matcher; `evaluation_steps` is now an opt-in cap, unset by
default, and `schema_resources` an opt-in cap beside it. The adapter and its consumers
configure no evaluator limit, and synthesized envelope contracts hold no pattern,
dynamic reference or unevaluated keyword, so none of that pin's new limits could refuse
them: the pattern total, counted over the forms the evaluator keeps for a contract's
patterns, the specialization limit, and the unevaluated-walk limit, which weighs each
reference by its kind. An explanation carries the library's first error with a fixed
message; the adapter reads no problem text, and its explanation test still expects
exactly one problem. Every other assertion kept its expected value, and the test
suite's run time was unchanged.

That pin also left the HTTP discovery attempt deadline to the application:
`ClientOptions::timeout` became an `Option<Duration>`, unset by default, and
`AttemptTimeout` and `DEFAULT_TIMEOUT_SECONDS` were removed. Neither the adapter nor
its consumers depend on the discovery crate, so the change asked nothing of them.

The next pin removed public concepts its API did not need, and the adapter
followed with one rename. Source inspection's `BindingFragment` is now `BindingValue`,
the same by-value binding a binding invoker receives, so an inspected target's binding
invokes as it is. Analysis builds each bindable target's binding
with it and leaves the author's `operation` and `source` keys absent, which the checked
`Inspector` now requires. `BindingValue::bind` returns a structural `BoundaryError`;
the inspection consumer's boxed error carries it unchanged. `SupportedKind` and
`KindSupportVerdict` are one pair of types, defined in the new internal crate
`openbindings-internal-interfaces` and reexported at each companion's existing path,
so a provider serving several jobs may return one advisory list from all three
traits. The adapter's three lists differ in their job-specific descriptions, and each
published interface lists the kinds supported for its own job, so each trait keeps its
own list over the shared `Kind::ALL` and every kind description is unchanged. Every
editing method now refuses with one `EditError`: `AuthoringError` and
`EditError::Authoring` are gone, a typed part's field collision is
`EditError::FieldCollision`, and a typed part over the byte budget is the ordinary
`Input` byte limit. Synthesis still converts an oversized metadata value into
`EditError::Input`. The operations and bindings it builds carry no additional fields,
and a supplied source whose additional field shadows a typed member still refuses as
`openbindings.synthesis.authoring`. A committed typed part over the byte budget now
reports `openbindings.synthesis.byte-limit` rather than
`openbindings.synthesis.authoring`, the code the editing-limit test already expects
for an oversized metadata value.

That pin's default evaluator host limits changed their settings, not the adapter's
results. `Limits::stack_bytes`, 256 KiB by default, replaces `evaluation_depth` and
still admits 1,024 nested evaluations at the default; `Limits::program_bytes`, 48 MiB
by default, replaces the dynamic specialization caps; and
`Limits::total_pattern_memory`, 64 MiB by default, bounds the regex crate's memory for
all of a contract's compiled patterns together. The adapter and its consumers
configure no evaluator limit, and synthesized envelope contracts are shallow and hold
no pattern or reference, so none of these refusals can reach them. The SDK's other
changes touch discovery, TypeScript, documentation, publication guards, tests and dead
code, none of which the adapter uses. Every existing assertion kept its expected
value, and the test suite's run time was unchanged.

The previous pin changed no public API and asked no adapter code change. Document
validation's memory now grows linearly with the document: the exact JSON arena keeps
fixed 32-byte nodes, one shared item list and one buffer of decoded escaped strings,
compared `$id` identifiers share runs of their texts, and the fixed checks read
strings in place. That pin changed no admission limit, and reports, schema programs
and evaluator outcomes stayed the same. The adapter's tests and consumers validate
synthesized and reloaded documents with `Document::validate`, and its qualification
consumers count live SDK JSON arenas through
`openbindings_internal_json::backend::live_arenas`; every assertion that owners return
to their baseline still holds, natively and in the Wasm probes. Core's document checks
now name each rule by its identifier internally, and rule evidence and findings stayed
the same. Every existing assertion kept its expected value.

The previous pin lowered the default JSON admission byte limit,
`JsonLimits::default().max_bytes`, from 64 MiB to 32 MiB; node and nesting limits are
unchanged. `Document::new`, `Document::parse`, `JsonValue::parse` and
`JsonValue::string` use the defaults, a document keeps the limits it was admitted
under for its edits, and that pin added `Document::parse_with_limits` and
`Document::limits`.
Contract projection keeps its fixed bounds. Under its default `Options` and the
client's default `Limits`, the adapter's own budgets sit below the new limit: source
content is held to `source_bytes` (16 MiB), a synthesized document to `source_bytes`
plus `result_bytes` (24 MiB) by the adapter's own `synthesis-result-limit`, and a 2xx
JSON response body to the client's `body_bytes` (4 MiB) before `JsonValue::parse`. A
synthesized document that exceeds 32 MiB, through raised budgets or large request
metadata, meets the SDK's default byte limit rather than one at 64 MiB and refuses as
`openbindings.synthesis.byte-limit`; a response body over 32 MiB, which needs a
raised client `body_bytes`, refuses as `ERR_EXECUTION_FAILED`. The editing-limit
test's 64 MiB description still exceeds the limit and keeps its code. Since that pin,
limit refusals name the setting to raise, input refusals read as sentences, OBI-10 findings come
one per failing meta-schema keyword at its value, and a pattern no-verdict carries the
pattern's location. The adapter and its consumers assert codes, kinds and the
`anchor-missing` location, never these messages, OBI-10 findings or pattern locations,
so every existing assertion kept its expected value.

This SDK revision makes the default evaluator's value checks faster with identical
outcomes: numbers and uniqueness are evaluated without boxed digit strings, and member
reads keep no per-name state. Its stack guard is now documented as a byte budget, so
the depth it admits for `unevaluated*` contracts can vary by a level with the build and
the host; the adapter configures no evaluator limit and its synthesized envelope
contracts are shallow, so no such contract reaches it. New owned-input calls,
`validate_document_owned`, `validate_document_owned_with_limits`,
`Document::parse_owned`, `Document::parse_owned_with_limits` and
`JsonValue::parse_owned_with_limits`, take a caller's buffer as the text instead of
copying it, with the borrowed forms' exact results; the SDK hands over the buffers it
builds itself, including for document edits and synthesized interfaces, so the
adapter's `Document::new` and edit batches benefit without a code change. The
TypeScript package's validation memory is lower, which the adapter's Rust-built
Node/Wasm probes do not use. The SDK's own manifests and lock move tokio to 1.54.1,
past RUSTSEC-2026-0386, in dev-dependencies and the discovery crate's optional
`native` feature, neither of which reaches the adapter's graph; the adapter keeps
tokio 1.53.3 through the client's exact pin, also past that advisory. The SDK's other
changes are its CI. The adapter and its consumers need no code change, and every
existing assertion keeps its expected value.

The client pin moved in the preceding change from
`fbcb868ea31a34fd4fd566244b7e40857f7df771`; its only effect on the graph is tokio
1.53.3, past RUSTSEC-2026-0386, because the client's reqwest companion pins tokio
exactly. The SDK carries no fork of any dependency. The locked
graph holds one registry identity each of `serde_json`, `jsonschema`,
`jsonschema-value` and `referencing`, at the SDK's versions, and none of the renamed
forks earlier SDK revisions carried; qualification refuses a graph that holds any of
those forks or lacks a single registry identity of one of those libraries. The
`regress` pattern engine has left the graph, since the SDK matches patterns on the
`regex` crate. Archive qualification packages the SDK's nine crates, including
`openbindings-internal-interfaces`, which an earlier pin added to the lock as a
dependency of the three companions. This pin moves only the SDK packages' source
revision: it adds or drops no crate, moves no registry package and changes no SDK
license or notice text. An earlier pin moved `rustls-webpki` to 0.103.16 and
`webpki-root-certs` to 1.0.10, past RUSTSEC-2026-0382 and RUSTSEC-2026-0383, which
reach the development graph through the reqwest transport. `tools/attribution.py`
regenerates both attribution inventories from cached locked sources:
`DEPENDENCIES.json` records the SDK source identities and notice hashes, and
`THIRD-PARTY-NOTICES.txt` reproduces the SDK packages' own license and notice texts at
the pinned commit.

These changes follow the pinned SDK's
[change log](https://github.com/openbindings/sdk/blob/3546fc29d3fb329656c6a7506f6006b9eb38375d/CHANGELOG.md),
[migration notes](https://github.com/openbindings/sdk/blob/3546fc29d3fb329656c6a7506f6006b9eb38375d/docs/migration.md)
and [dependency maintenance contract](https://github.com/openbindings/sdk/blob/3546fc29d3fb329656c6a7506f6006b9eb38375d/DEPENDENCY-MAINTENANCE.md).

The table below records the original extraction dispositions, not a claim that
migrated consumers remain byte-identical to that import.

| Imported file | SDK source at the pinned commit | Disposition |
| --- | --- | --- |
| `.gitattributes` | `.gitattributes` | rewritten: Retain the CRUD fixture byte-preservation rule. |
| `.gitignore` | `.gitignore` | rewritten: Ignore this repository's build, Node and Python generated files. |
| `Cargo.lock` | `Cargo.lock` | rewritten: Resolve pinned Git SDK packages and the moved workspace members offline, preserving baseline registry versions. |
| `Cargo.toml` | `Cargo.toml` | rewritten: Declare the independent workspace, repository metadata and preserved build policy. |
| `LICENSE` | `LICENSE` | identical |
| `README.md` | `crates/openbindings-openapi/README.md` | rewritten: Present the existing profile and limits in the independent repository with current use and verification paths. |
| `crates/openbindings-openapi/Cargo.toml` | `crates/openbindings-openapi/Cargo.toml` | rewritten: Pin SDK Git dependencies and adapt workspace or local adapter paths. |
| `crates/openbindings-openapi/README.md` | `crates/openbindings-openapi/README.md` | rewritten: Update current repository, verification and documentation paths; remove links to archived records. |
| `crates/openbindings-openapi/src/analysis.rs` | `crates/openbindings-openapi/src/analysis.rs` | identical |
| `crates/openbindings-openapi/src/invocation.rs` | `crates/openbindings-openapi/src/invocation.rs` | identical |
| `crates/openbindings-openapi/src/lib.rs` | `crates/openbindings-openapi/src/lib.rs` | identical |
| `crates/openbindings-openapi/src/security.rs` | `crates/openbindings-openapi/src/security.rs` | identical |
| `crates/openbindings-openapi/src/synthesis.rs` | `crates/openbindings-openapi/src/synthesis.rs` | identical |
| `crates/openbindings-openapi/tests/auth_client.rs` | `crates/openbindings-openapi/tests/auth_client.rs` | identical |
| `crates/openbindings-openapi/tests/auth_http.rs` | `crates/openbindings-openapi/tests/auth_http.rs` | identical |
| `crates/openbindings-openapi/tests/crud.rs` | `crates/openbindings-openapi/tests/crud.rs` | identical |
| `crates/openbindings-openapi/tests/fixtures/README.md` | `crates/openbindings-openapi/tests/fixtures/README.md` | rewritten: Update current repository, verification and documentation paths; remove links to archived records. |
| `crates/openbindings-openapi/tests/fixtures/auth/manual-3.0.4.obi.json` | `crates/openbindings-openapi/tests/fixtures/auth/manual-3.0.4.obi.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/auth/manual-3.1.2.obi.json` | `crates/openbindings-openapi/tests/fixtures/auth/manual-3.1.2.obi.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/auth/parts-3.0.4.json` | `crates/openbindings-openapi/tests/fixtures/auth/parts-3.0.4.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/auth/parts-3.1.2.json` | `crates/openbindings-openapi/tests/fixtures/auth/parts-3.1.2.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/auth/service-3.0.4.json` | `crates/openbindings-openapi/tests/fixtures/auth/service-3.0.4.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/auth/service-3.1.2.json` | `crates/openbindings-openapi/tests/fixtures/auth/service-3.1.2.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/auth/variants-3.0.4.json` | `crates/openbindings-openapi/tests/fixtures/auth/variants-3.0.4.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/auth/variants-3.1.2.json` | `crates/openbindings-openapi/tests/fixtures/auth/variants-3.1.2.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/auth/wire-oracle.json` | `crates/openbindings-openapi/tests/fixtures/auth/wire-oracle.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/contrast-20.json` | `crates/openbindings-openapi/tests/fixtures/contrast-20.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/contrast-30.json` | `crates/openbindings-openapi/tests/fixtures/contrast-30.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/contrast-32.json` | `crates/openbindings-openapi/tests/fixtures/contrast-32.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/duplicate-ids.json` | `crates/openbindings-openapi/tests/fixtures/duplicate-ids.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/empty.json` | `crates/openbindings-openapi/tests/fixtures/empty.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/future-patch.json` | `crates/openbindings-openapi/tests/fixtures/future-patch.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/identity-31.json` | `crates/openbindings-openapi/tests/fixtures/identity-31.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/invalid-paths.json` | `crates/openbindings-openapi/tests/fixtures/invalid-paths.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/malformed-neighbor-31.json` | `crates/openbindings-openapi/tests/fixtures/malformed-neighbor-31.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/malformed-only.json` | `crates/openbindings-openapi/tests/fixtures/malformed-only.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/method-conflict.json` | `crates/openbindings-openapi/tests/fixtures/method-conflict.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/minimal-31.json` | `crates/openbindings-openapi/tests/fixtures/minimal-31.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/minimal-31.yaml` | `crates/openbindings-openapi/tests/fixtures/minimal-31.yaml` | identical |
| `crates/openbindings-openapi/tests/fixtures/missing-resource-31.json` | `crates/openbindings-openapi/tests/fixtures/missing-resource-31.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/mounted-31.json` | `crates/openbindings-openapi/tests/fixtures/mounted-31.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/destinations.json` | `crates/openbindings-openapi/tests/fixtures/r1/destinations.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/destinations.obi.json` | `crates/openbindings-openapi/tests/fixtures/r1/destinations.obi.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/expected-caller.json` | `crates/openbindings-openapi/tests/fixtures/r1/expected-caller.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/mounted.json` | `crates/openbindings-openapi/tests/fixtures/r1/mounted.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/mounted.obi.json` | `crates/openbindings-openapi/tests/fixtures/r1/mounted.obi.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/nested-entry.json` | `crates/openbindings-openapi/tests/fixtures/r1/nested-entry.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/nested-parameters.json` | `crates/openbindings-openapi/tests/fixtures/r1/nested-parameters.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/nested-parts.json` | `crates/openbindings-openapi/tests/fixtures/r1/nested-parts.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/parcels.json` | `crates/openbindings-openapi/tests/fixtures/r1/parcels.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/parcels.obi.json` | `crates/openbindings-openapi/tests/fixtures/r1/parcels.obi.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/parts.json` | `crates/openbindings-openapi/tests/fixtures/r1/parts.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/r1/wire-oracle.json` | `crates/openbindings-openapi/tests/fixtures/r1/wire-oracle.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/readiness-cases.json` | `crates/openbindings-openapi/tests/fixtures/readiness-cases.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/schema-boundary-31.json` | `crates/openbindings-openapi/tests/fixtures/schema-boundary-31.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/subordinate-defects.json` | `crates/openbindings-openapi/tests/fixtures/subordinate-defects.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/supplied-paths-31.json` | `crates/openbindings-openapi/tests/fixtures/supplied-paths-31.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/unknown-path-field.json` | `crates/openbindings-openapi/tests/fixtures/unknown-path-field.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/v30/entry.json` | `crates/openbindings-openapi/tests/fixtures/v30/entry.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/v30/exchanges.json` | `crates/openbindings-openapi/tests/fixtures/v30/exchanges.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/v30/manual.obi.json` | `crates/openbindings-openapi/tests/fixtures/v30/manual.obi.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/v30/parameters.json` | `crates/openbindings-openapi/tests/fixtures/v30/parameters.json` | identical |
| `crates/openbindings-openapi/tests/fixtures/v30/routes.json` | `crates/openbindings-openapi/tests/fixtures/v30/routes.json` | identical |
| `crates/openbindings-openapi/tests/manual.rs` | `crates/openbindings-openapi/tests/manual.rs` | identical |
| `crates/openbindings-openapi/tests/semantics.rs` | `crates/openbindings-openapi/tests/semantics.rs` | identical |
| `crates/openbindings-openapi/tests/synthesis.rs` | `crates/openbindings-openapi/tests/synthesis.rs` | identical |
| `crates/openbindings-openapi/tests/versions.rs` | `crates/openbindings-openapi/tests/versions.rs` | identical |
| `examples/openapi-inspection-consumer/Cargo.toml` | `examples/openapi-inspection-consumer/Cargo.toml` | rewritten: Pin SDK Git dependencies and adapt workspace or local adapter paths. |
| `examples/openapi-inspection-consumer/README.md` | `examples/openapi-inspection-consumer/README.md` | rewritten: Update current repository, verification and documentation paths; remove links to archived records. |
| `examples/openapi-inspection-consumer/examples/author.rs` | `examples/openapi-inspection-consumer/examples/author.rs` | rewritten: Repair the retired kind, text/retrievalUri carrier and path/method binding; retain independent recovery and preparation coverage. |
| `examples/openapi-inspection-consumer/examples/measure.rs` | `examples/openapi-inspection-consumer/examples/measure.rs` | rewritten: Repair the retired kind, text/retrievalUri carrier and path/method binding; retain independent recovery and preparation coverage. |
| `examples/openapi-inspection-consumer/src/lib.rs` | `examples/openapi-inspection-consumer/src/lib.rs` | rewritten: Repair the retired kind, text/retrievalUri carrier and path/method binding; retain independent recovery and preparation coverage. |
| `examples/openapi-inspection-consumer/tests/consumer.rs` | `examples/openapi-inspection-consumer/tests/consumer.rs` | rewritten: Repair the retired kind, text/retrievalUri carrier and path/method binding; retain independent recovery and preparation coverage. |
| `examples/openapi-inspection-consumer/wasm-probe.ts` | `examples/openapi-inspection-consumer/wasm-probe.ts` | rewritten: Repair the retired kind, text/retrievalUri carrier and path/method binding; retain independent recovery and preparation coverage. |
| `examples/openapi-invocation-only/Cargo.toml` | `examples/openapi-invocation-only/Cargo.toml` | rewritten: Pin SDK Git dependencies and adapt workspace or local adapter paths. |
| `examples/openapi-invocation-only/src/lib.rs` | `examples/openapi-invocation-only/src/lib.rs` | identical |
| `examples/openapi-json-path-consumer/Cargo.toml` | `examples/openapi-json-path-consumer/Cargo.toml` | rewritten: Pin SDK Git dependencies and adapt workspace or local adapter paths. |
| `examples/openapi-json-path-consumer/README.md` | `examples/openapi-json-path-consumer/README.md` | rewritten: Update current repository, verification and documentation paths; remove links to archived records. |
| `examples/openapi-json-path-consumer/examples/auth_diagnostics.rs` | `examples/openapi-json-path-consumer/examples/auth_diagnostics.rs` | identical |
| `examples/openapi-json-path-consumer/examples/auth_measure.rs` | `examples/openapi-json-path-consumer/examples/auth_measure.rs` | identical |
| `examples/openapi-json-path-consumer/examples/auth_resolution.rs` | `examples/openapi-json-path-consumer/examples/auth_resolution.rs` | identical |
| `examples/openapi-json-path-consumer/examples/measure.rs` | `examples/openapi-json-path-consumer/examples/measure.rs` | identical |
| `examples/openapi-json-path-consumer/src/auth-fixtures/manual-3.0.4.obi.json` | `examples/openapi-json-path-consumer/src/auth-fixtures/manual-3.0.4.obi.json` | identical |
| `examples/openapi-json-path-consumer/src/auth-fixtures/manual-3.1.2.obi.json` | `examples/openapi-json-path-consumer/src/auth-fixtures/manual-3.1.2.obi.json` | identical |
| `examples/openapi-json-path-consumer/src/auth-fixtures/parts-3.0.4.json` | `examples/openapi-json-path-consumer/src/auth-fixtures/parts-3.0.4.json` | identical |
| `examples/openapi-json-path-consumer/src/auth-fixtures/parts-3.1.2.json` | `examples/openapi-json-path-consumer/src/auth-fixtures/parts-3.1.2.json` | identical |
| `examples/openapi-json-path-consumer/src/auth-fixtures/service-3.0.4.json` | `examples/openapi-json-path-consumer/src/auth-fixtures/service-3.0.4.json` | identical |
| `examples/openapi-json-path-consumer/src/auth-fixtures/service-3.1.2.json` | `examples/openapi-json-path-consumer/src/auth-fixtures/service-3.1.2.json` | identical |
| `examples/openapi-json-path-consumer/src/auth-fixtures/variants-3.0.4.json` | `examples/openapi-json-path-consumer/src/auth-fixtures/variants-3.0.4.json` | identical |
| `examples/openapi-json-path-consumer/src/auth-fixtures/variants-3.1.2.json` | `examples/openapi-json-path-consumer/src/auth-fixtures/variants-3.1.2.json` | identical |
| `examples/openapi-json-path-consumer/src/auth-fixtures/wire-oracle.json` | `examples/openapi-json-path-consumer/src/auth-fixtures/wire-oracle.json` | identical |
| `examples/openapi-json-path-consumer/src/auth.rs` | `examples/openapi-json-path-consumer/src/auth.rs` | identical |
| `examples/openapi-json-path-consumer/src/auth_acceptance.rs` | `examples/openapi-json-path-consumer/src/auth_acceptance.rs` | identical |
| `examples/openapi-json-path-consumer/src/crud-fixtures/exchanges.json` | `examples/openapi-json-path-consumer/src/crud-fixtures/exchanges.json` | identical |
| `examples/openapi-json-path-consumer/src/crud-fixtures/manual-3.0.4.obi.json` | `examples/openapi-json-path-consumer/src/crud-fixtures/manual-3.0.4.obi.json` | identical |
| `examples/openapi-json-path-consumer/src/crud-fixtures/manual-3.1.2.obi.json` | `examples/openapi-json-path-consumer/src/crud-fixtures/manual-3.1.2.obi.json` | identical |
| `examples/openapi-json-path-consumer/src/crud-fixtures/service-3.0.4.json` | `examples/openapi-json-path-consumer/src/crud-fixtures/service-3.0.4.json` | identical |
| `examples/openapi-json-path-consumer/src/crud-fixtures/service-3.1.2.json` | `examples/openapi-json-path-consumer/src/crud-fixtures/service-3.1.2.json` | identical |
| `examples/openapi-json-path-consumer/src/crud.rs` | `examples/openapi-json-path-consumer/src/crud.rs` | identical |
| `examples/openapi-json-path-consumer/src/lib.rs` | `examples/openapi-json-path-consumer/src/lib.rs` | identical |
| `examples/openapi-json-path-consumer/src/wasm.rs` | `examples/openapi-json-path-consumer/src/wasm.rs` | identical |
| `examples/openapi-json-path-consumer/tests/consumer.rs` | `examples/openapi-json-path-consumer/tests/consumer.rs` | identical |
| `examples/openapi-json-path-consumer/tests/crud.rs` | `examples/openapi-json-path-consumer/tests/crud.rs` | identical |
| `examples/openapi-json-path-consumer/wasm-probe.ts` | `examples/openapi-json-path-consumer/wasm-probe.ts` | identical |
| `package-lock.json` | `packages/typescript/package-lock.json` | rewritten: Retain only the identical TypeScript 5.9.3 resolution and integrity for this private development package. |
| `rust-toolchain.toml` | `rust-toolchain.toml` | identical |
| `tools/verify-openapi-inspection.py` | `tools/verify-openapi-inspection.py` | rewritten: Qualify independent Git/archive consumers through the shared source-identity checks and local Node tooling. |
| `tools/verify-openapi-invocation-only.py` | `tools/verify-openapi-invocation-only.py` | rewritten: Qualify independent Git/archive consumers through the shared source-identity checks and local Node tooling. |
| `tools/verify-openapi-json-path.py` | `tools/verify-openapi-json-path.py` | rewritten: Qualify independent Git/archive consumers through the shared source-identity checks and local Node tooling. |
