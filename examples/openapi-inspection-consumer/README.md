# Independent OpenAPI inspection consumer

`src/lib.rs` uses the public SDK packages to inspect, select, author, validate and
reload an OBI. Tests independently create a binding and prove direct/referenced
mounted-target correspondence using the public OpenAPI client. The author owns
operation/source keys; unknown schema dialect and exact numeric content survive.
Sources use `EXPERIMENTAL_KIND`, JSON text in `document`, and optional `retrievalUri`.
Binding content uses `path` and lowercase `method`; inspection `sourceRef` remains
the source-local pointer used for target selection.

Run `python3 tools/verify-openapi-inspection.py --wasm` from the repository root.
For extracted packages, add `--archives --sdk-source <cached-sdk-checkout>` at the
pinned SDK revision. Source mode resolves SDK dependencies through pinned Git sources.
`openbindings-internal-json` is used solely for qualification owner counters;
consumer behavior uses public SDK APIs. The small Wasm export is test plumbing.
`examples/measure.rs` is preserved as a stopped measurement example, compiled but
never executed by qualification or CI.
