# Clean import provenance

Imported from SDK commit `78724ae`, full revision
`78724aedca8d0dc104aa3be6254b4a225b7e9b58` in
[openbindings/sdk](https://github.com/openbindings/sdk).
This repository receives one clean source import, with no filtered history.
Historical adapter evidence and role-loop records are archived outside both product
repositories. At import, all adapter `src/` and test `.rs` files and all fixture bytes were identical.
The stopped measurement examples remain sources, compiled but never executed by CI.

The root development manifest, CI workflow, shared qualification helper and
dependency attribution files are new extraction plumbing. The consumer test binary
serializes its own tests with a local lock because they count process-global owners.
`NOTICE` describes this resolved graph. No SDK implementation is copied into this
product repository. Dependencies are immutable Git pins: the SDK at
`f82a5135aabb146b398553f0344d107bea7f52ec` (vocabulary, S6a and S6b working-draft contracts)
and the client at
`62d0a05ee9ff3f01512a2e0855a449157dbd9d45`.

The subsequent SDK migration to `f82a5135aabb146b398553f0344d107bea7f52ec`
adapts SDK vocabulary and cancellation signatures while preserving the adapter
profile and client pin. Synchronous helpers observe `&dyn WorkControl`; role
providers own their companion's `CancellationToken`. The warm validated-invocation
measurement retains a caller-owned `PreparedOperation` and uses `Attempt` to
preserve compiled-contract reuse after removal of the core preparation cache.
The adapter's separate bounded source-analysis cache is retained. The table below
records the original extraction dispositions.

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
| `crates/openbindings-openapi/tests/fixtures/r1/experiment-contract.json` | `crates/openbindings-openapi/tests/fixtures/r1/experiment-contract.json` | identical |
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
