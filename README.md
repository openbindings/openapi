The OpenBindings implementation of the source-inspection, interface-synthesis and binding-invocation interfaces for OpenAPI, built on [`openapi-client`](https://github.com/openbindings/openapi-client).

The packages are unpublished. This repository depends on the SDK at
`a8e40f340b6195acc4718b150137ab899e5d65e2` and the independent client at
`fbcb868ea31a34fd4fd566244b7e40857f7df771`. The SDK defines the interfaces and supplies
no kind-specific instances. This project supplies the OpenAPI provider.

Use `OpenApi::default()` with the SDK's checked `Inspector` or `Synthesizer`.
Use `OpenApi::with_transport` with application-owned execution and the checked
binding or operation invoker. The [JSON-path consumer](examples/openapi-json-path-consumer/)
shows inspection, synthesis, save/reload and invocation. The independent
[inspection consumer](examples/openapi-inspection-consumer/) covers manual authoring
and supplied-resource recovery, and the [minimal consumer](examples/openapi-invocation-only/)
guards dependency isolation.

The private `local.openapi-json-probe.r1` and `local.openapi30-json-probe.r1` kinds
retain their meanings. Sources carry exact JSON text in `document` and optional
`retrievalUri`; bindings carry `path` and lowercase `method`. Reloaded documents
must pass SDK validation before operation invocation or
value-contract preparation. The current SDK pin uses one exact `Document`, `Conformant<Document>` proof with infallible typed
views, value contracts handed out by conformant operations and companion-owned
`CancellationToken` interface signatures. Value checks report the verdict alone unless
the caller asks for an explanation; [migration details](PROVENANCE.md) describe the
diagnostic and reference boundaries. It implements no
published OpenAPI binding standard.

The existing finite JSON profile admits GET, POST, PUT, PATCH and DELETE for OpenAPI
3.0 and 3.1, supplied references, scalar path/query parameters, finite JSON bodies,
empty declared successes, and caller-supplied bearer and header API-key credentials.
No schema-constraint translation, security acquisition, implicit fetching, automatic
retry or full OpenAPI coverage is claimed. HEAD/OPTIONS/TRACE, richer encodings,
streaming/binary delivery and literal Unicode path templates remain outside the
invocation profile. See the [complete package guide](crates/openbindings-openapi/README.md)
for kind contracts, budgets, security policy, synthesis loss and refusal behavior.

The incremental-auth performance check remains unresolved and deferred with
its original budgets unchanged. The stopped measurement examples are compiled and
linted but never run by CI. This repository carries no historical loop evidence.
Native HTTP and local Node/Wasm probes do not establish browser/Worker network
support or a released TypeScript interface API.

Use Rust 1.99.0 with the wasm32-unknown-unknown target, wasm-bindgen-cli 0.2.129,
Node 22.19.0, npm and Python 3.13+. With dependency caches populated:

```sh
cargo build --locked --offline
cargo test --locked --offline
npm ci --offline --ignore-scripts
python3 tools/attribution.py --check
python3 tools/verify-openapi-inspection.py --wasm
python3 tools/verify-openapi-json-path.py --wasm
python3 tools/verify-openapi-invocation-only.py
```

The verifiers generate independent consumers under `target/`, preserve selected
lockfile versions, and check a single source identity for each SDK package and the
exact client revision. They require no SDK checkout in the source-Git mode.
Archive qualification packages only an explicitly supplied clean cached SDK source:

```sh
python3 tools/verify-openapi-json-path.py --archives --wasm --sdk-source <cached-sdk-checkout>
python3 tools/verify-openapi-inspection.py --archives --wasm --sdk-source <cached-sdk-checkout>
python3 tools/verify-openapi-invocation-only.py --archives
```

The SDK source must have the pinned tree, unchanged tracked files and no tracked
symlinks. The pinned revision, checkout commit and tree identity are recorded in the generated receipt.
Extracted SDK packages are patched for both their original Git dependencies and
normalized registry dependencies; no source checkout may escape into the archive
consumer graph. The minimal archive consumer checks the preceding JSON-path
receipt's input and extracted-package hashes before using that snapshot.

CI requires full adapter tests on Linux, macOS and Windows, the independent
consumers, extracted Cargo archives, and both Node/Wasm probes. CI also runs
`cargo deny check advisories`; vulnerabilities and yanked versions fail, and every
advisory ignore requires an ID and a written reason in `deny.toml`. The fixture-specific
`-text` attribute preserves literal CRUD exchanges on every host.
Regenerate `DEPENDENCIES.json` and `THIRD-PARTY-NOTICES.txt` after dependency changes
with `python3 tools/attribution.py`; CI checks both against the offline locked
Cargo graph and installed pinned TypeScript sources. Their `-text` attributes
preserve the generated bytes on every host. Qualification consumers alone enable
the client's non-default `qualification` feature for owner counters.
[PROVENANCE.md](PROVENANCE.md) maps the clean import to SDK source paths.
