#!/usr/bin/env python3
"""Regenerate the adapter's attribution inventories offline; --check never writes.

Following the SDK generator, derive metadata and hashes from locked, cached
sources, never from the generated inventories. Preserve the adapter's original
scope and serialization: all resolved Cargo packages except workspace members,
including development/platform packages, sorted by (name, version), followed by
the pinned npm TypeScript development tool. No platform filter is applied.

Select top-level LICENSE/COPYING/COPYRIGHT/NOTICE files case-insensitively, plus
the SDK schema crate's embedded schemas/LICENSE files. Git packages without local texts
use their checkout's root LICENSE under repository/LICENSE. Packages with no
source texts retain an empty notices list, as in the original inventory; this
does not assert that their declared license expression replaces those texts.
TypeScript contributes LICENSE.txt and ThirdPartyNoticeText.txt. Hash original
bytes; normalize CRLF/CR to LF and strip trailing whitespace only when rendering
notice bodies. JSON uses two-space indentation, ASCII escapes and a final LF.
Notice headings and sections retain the adapter's original byte format.
Acquire sources with cargo fetch --locked and npm ci --ignore-scripts first.
"""

import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
SCOPE = (
    "Resolved Cargo graph, including development and platform-specific packages, "
    "plus the pinned npm development tool; not a runtime-only claim."
)
PREAMBLE = "OpenBindings OpenAPI implementation dependency attribution\n"


def json_bytes(value):
    return (json.dumps(value, indent=2, ensure_ascii=True) + "\n").encode("utf-8")


def metadata():
    command = ["cargo", "metadata", "--offline", "--locked", "--format-version", "1"]
    env = os.environ.copy()
    env["CARGO_NET_OFFLINE"] = "true"
    result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
    if result.returncode:
        raise ValueError("Cargo metadata failed:\n" + result.stderr.rstrip())
    return json.loads(result.stdout)


def is_notice(path):
    return path.is_file() and re.match(r"^(license|copying|copyright|notice)", path.name, re.I)


def source_notices(package):
    source = Path(package["manifest_path"]).resolve().parent
    paths = [(p.name, p) for p in source.iterdir() if is_notice(p)]
    if package["name"] == "openbindings-internal-schema":
        paths.extend(
            (p.relative_to(source).as_posix(), p)
            for p in (source / "schemas").rglob("*")
            if p.is_file() and p.name.upper().startswith("LICENSE")
        )
    if not paths and package["source"].startswith("git+"):
        checkout = source
        while not (checkout / ".git").exists():
            if checkout.parent == checkout:
                raise ValueError(f"no Git checkout root for {source}")
            checkout = checkout.parent
        license_path = checkout / "LICENSE"
        if not license_path.is_file():
            raise ValueError(f"no repository LICENSE for {package['name']}")
        paths.append(("repository/LICENSE", license_path))
    return sorted(paths)


def package_record(package, paths):
    notices, bodies = [], {}
    for relative, path in paths:
        data = path.read_bytes()
        notices.append({"path": relative, "sha256": hashlib.sha256(data).hexdigest()})
        bodies[relative] = data.decode("utf-8").replace("\r\n", "\n").replace("\r", "\n").rstrip()
    return {
        "name": package["name"],
        "version": package["version"],
        "declared_license": package["license"],
        "repository": package["repository"],
        "source": package["source"],
        "notices": notices,
    }, bodies


def typescript_record():
    lock = json.loads((ROOT / "package-lock.json").read_text(encoding="utf-8"))
    spec = lock["packages"]["node_modules/typescript"]
    authored = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))
    source = ROOT / "node_modules/typescript"
    installed = json.loads((source / "package.json").read_text(encoding="utf-8"))
    if spec["version"] != authored["devDependencies"]["typescript"] or installed["version"] != spec["version"]:
        raise ValueError("TypeScript sources do not match the exact development pin; run npm ci --offline --ignore-scripts")
    repository = installed["repository"]
    if isinstance(repository, dict):
        repository = repository["url"]
    repository = re.sub(r"^git\+", "", repository).removesuffix(".git")
    return package_record({
        "name": installed["name"], "version": spec["version"],
        "license": installed["license"], "repository": repository,
        "source": "npm development dependency",
    }, [(name, source / name) for name in ("LICENSE.txt", "ThirdPartyNoticeText.txt")])


def generate():
    graph = metadata()
    members = set(graph["workspace_members"])
    packages = sorted(
        (p for p in graph["packages"] if p["id"] not in members),
        key=lambda p: (p["name"], p["version"]),
    )
    items = [package_record(p, source_notices(p)) for p in packages]
    items.append(typescript_record())
    notices = PREAMBLE
    for record, bodies in items:
        for notice in record["notices"]:
            relative = notice["path"]
            notices += f"\n===== {record['name']} {record['version']}: {relative} =====\n"
            notices += bodies[relative] + "\n"
    return {
        Path("DEPENDENCIES.json"): json_bytes({"scope": SCOPE, "packages": [r for r, _ in items]}),
        Path("THIRD-PARTY-NOTICES.txt"): notices.encode("utf-8"),
    }


def report_difference(path, actual, expected):
    if actual is None:
        print(f"STALE {path}: missing file; expected {len(expected)} bytes", file=sys.stderr)
        return
    offset = next((i for i, (a, b) in enumerate(zip(actual, expected)) if a != b), min(len(actual), len(expected)))
    line = actual[:offset].count(b"\n") + 1
    column = offset - actual.rfind(b"\n", 0, offset)
    print(
        f"STALE {path}: first difference at byte {offset} (line {line}, column {column}); "
        f"checked-in {len(actual)} bytes, expected {len(expected)} bytes", file=sys.stderr,
    )
    changes = list(difflib.unified_diff(
        actual.decode("utf-8", errors="replace").splitlines(keepends=True),
        expected.decode("utf-8", errors="replace").splitlines(keepends=True),
        fromfile=str(path) + " (checked-in)", tofile=str(path) + " (generated)", n=3,
    ))
    for change in changes[:80]:
        sys.stderr.write(change if change.endswith("\n") else change + "\n\\ No newline at end of file\n")
    if len(changes) > 80:
        print(f"... {len(changes) - 80} more diff lines omitted", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Fail if any inventory differs; do not write files")
    args = parser.parse_args()
    try:
        stale = []
        for path, expected in generate().items():
            destination = ROOT / path
            actual = destination.read_bytes() if destination.exists() else None
            if actual != expected:
                stale.append(path)
                if args.check:
                    report_difference(path, actual, expected)
                else:
                    destination.write_bytes(expected)
                    print(f"Wrote {path}")
        if args.check and stale:
            print(f"{len(stale)} stale attribution file(s); refresh with python3 tools/attribution.py", file=sys.stderr)
            return 1
        print("Both attribution files match the offline locked dependency graph.")
        return 0
    except (OSError, ValueError, KeyError) as error:
        print(f"Attribution error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
