# Inspection corpus

The eleven `*-31`, contrast and supplied-resource files are the unchanged,
project-authored 2026-10-12 readiness seeds. `readiness-cases.json` retains their
original expected results, provenance and SHA-256 digests as historical evidence.
Current acceptance outcomes are in `../inspection.rs`, including the AD07 draft
correction exercised directly by the maintained regression tests.

Additional small files exercise duplicate IDs, definitively malformed-only
operations, subordinate defects, empty/future-version sources, invalid paths, and
known client Path Item limitations. Inline variants in the tests cover carrier
admission, missing versions, later-edition field leakage, budgets and cancellation.
All are authored for this repository; no network or external API is used.
