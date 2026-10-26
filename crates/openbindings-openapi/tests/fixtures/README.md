# Test fixtures

The adapter tests read the descriptions, hand-authored OBI documents and recorded
exchanges under `parcels/` and `v30/`, and the authenticated service descriptions and
wire expectations under `auth/`; each test names the files it reads. `v30/entry.json` is
the readable source of the description `v30/manual.obi.json` embeds as text. The other
`auth/` files are identical copies of the JSON-path consumer's authentication fixtures,
which that consumer reads from its own directory.

The eleven `*-31`, contrast and supplied-resource files are the unchanged,
project-authored 2026-10-12 readiness seeds. `readiness-cases.json` keeps their original
expected results, provenance and SHA-256 digests as a historical record; its wording is
that record's and does not describe current behavior. Of the seeds, the maintained tests
read only `minimal-31.yaml`, as a YAML document this implementation refuses.

The other small files (duplicate IDs, malformed-only operations, subordinate defects,
empty and future-version sources, invalid paths and known client Path Item limitations)
are authored inputs that no maintained test reads. Inline variants in the tests cover
carrier admission, missing versions, later-edition field leakage, budgets and
cancellation. All are authored for this repository; no network or external API is used.
