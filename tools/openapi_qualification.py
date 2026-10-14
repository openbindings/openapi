"""Shared offline consumer qualification, with explicit source identity checks."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
SDK_URL = "https://github.com/openbindings/sdk"
SDK_REV = "f82a5135aabb146b398553f0344d107bea7f52ec"
CLIENT_URL = "https://github.com/openbindings/openapi-client"
CLIENT_REV = "7de316b1ec5d970284cf244ec38eb56b565019d8"
SDK_PATHS = [
    "crates/openbindings", "crates/openbindings-internal-json",
    "crates/openbindings-internal-cancellation",
    "crates/openbindings-source-inspection", "crates/openbindings-interface-synthesis",
    "crates/openbindings-invocation", "crates/openbindings-json-schema-evaluator",
    "vendor/openbindings-internal-jsonschema", "vendor/openbindings-internal-jsonschema-value",
    "vendor/openbindings-internal-serde-json", "vendor/openbindings-internal-referencing",
    "vendor/openbindings-internal-regress",
]
SDK_NAMES = {Path(p).name for p in SDK_PATHS}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def input_hashes():
    files = [ROOT / p for p in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "package.json", "package-lock.json", ".gitattributes"]]
    for directory in ["crates", "examples", "tools", "qualification/inputs"]:
        files.extend(p for p in (ROOT / directory).rglob("*") if p.is_file() and "__pycache__" not in p.parts)
    return {p.relative_to(ROOT).as_posix(): sha(p) for p in sorted(files)}


def check_pins():
    for manifest in [ROOT / "crates/openbindings-openapi/Cargo.toml", *sorted((ROOT / "examples").glob("*/Cargo.toml"))]:
        data = tomllib.loads(manifest.read_text())
        for section in [data, *data.get("target", {}).values()]:
            for kind in ["dependencies", "dev-dependencies", "build-dependencies"]:
                for alias, spec in section.get(kind, {}).items():
                    name = spec.get("package", alias) if isinstance(spec, dict) else alias
                    if name in SDK_NAMES:
                        if not isinstance(spec, dict) or spec.get("git") != SDK_URL or spec.get("rev") != SDK_REV or "path" in spec:
                            raise SystemExit(f"Unpinned SDK dependency: {manifest.relative_to(ROOT)}: {alias}")
                    if name.startswith("dynamic-openapi-client"):
                        if not isinstance(spec, dict) or spec.get("git") != CLIENT_URL or spec.get("rev") != CLIENT_REV or "path" in spec:
                            raise SystemExit(f"Unpinned client dependency: {manifest.relative_to(ROOT)}: {alias}")


def check_metadata(metadata, sources=None):
    sources = sources or {}
    identities = {}
    for package in metadata["packages"]:
        name = package["name"]
        if name not in SDK_NAMES and not name.startswith("dynamic-openapi-client") and name != "openbindings-openapi":
            continue
        if name in identities:
            raise SystemExit(f"Multiple source identities for {name}")
        identities[name] = package.get("source") or Path(package["manifest_path"]).resolve().parent.as_posix()
        path = Path(package["manifest_path"]).resolve().parent
        if name in sources:
            if path != sources[name].resolve():
                raise SystemExit(f"Archive consumer resolved a checkout escape: {name}")
        elif name in SDK_NAMES:
            if package.get("source") != f"git+{SDK_URL}?rev={SDK_REV}#{SDK_REV}":
                raise SystemExit(f"Unexpected SDK source: {name}")
        elif name.startswith("dynamic-openapi-client"):
            if package.get("source") != f"git+{CLIENT_URL}?rev={CLIENT_REV}#{CLIENT_REV}":
                raise SystemExit(f"Unexpected client source: {name}")
        elif path != ROOT / "crates/openbindings-openapi":
            raise SystemExit("Unexpected adapter source")
    if "dynamic-openapi-client" not in identities:
        raise SystemExit("Pinned client missing from resolved graph")
    return identities


def verified_sdk_source(source):
    if source is None:
        raise SystemExit("Archive qualification requires --sdk-source pointing to a clean cached checkout at " + SDK_REV)
    source = source.resolve()
    head = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if head != SDK_REV:
        raise SystemExit(f"SDK source HEAD is {head}; expected {SDK_REV}")
    subprocess.run(["git", "-C", str(source), "diff", "--exit-code", "HEAD", "--"], check=True, capture_output=True)
    # Cargo packages only tracked inputs. Reject tracked symlinks that could leave
    # the checked tree, and record its identity without modifying the cache.
    files = subprocess.check_output(["git", "-C", str(source), "ls-files", "-z"], text=True).split("\0")
    for name in filter(None, files):
        if (source / name).is_symlink():
            raise SystemExit("SDK source contains a tracked symlink: " + name)
    tree = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD^{tree}"], text=True).strip()
    return source, {"commit": head, "tree": tree}


def extract_fresh(archive, destination):
    with tarfile.open(archive) as packed:
        packed.extractall(destination, filter="data")
        for member in packed.getmembers():
            if member.isfile():
                os.utime(destination / member.name, None)


def consumer_manifest(fixture, adapter):
    text = (fixture / "Cargo.toml").read_text()
    package = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]
    for key in ["edition", "rust-version", "license", "repository", "publish"]:
        value = json.dumps(package[key])
        text = text.replace(f"{key}.workspace = true", f"{key} = {value}")
    text = text.replace("[lints]\nworkspace = true", '[lints.rust]\nunsafe_code = "forbid"')
    text = text.replace('path = "../../crates/openbindings-openapi"', "path = " + json.dumps(str(adapter)))
    return text + "\n[workspace]\n"


def patch_table(sources):
    rows = "".join(f"{name} = {{ path = {json.dumps(str(path))} }}\n" for name, path in sorted(sources.items()) if name in SDK_NAMES)
    clients = "".join(f'{name} = {{ git = "{CLIENT_URL}", rev = "{CLIENT_REV}" }}\n' for name in ["dynamic-openapi-client", "dynamic-openapi-client-reqwest"])
    return '\n[patch.crates-io]\n' + rows + clients + f'\n[patch."{SDK_URL}"]\n' + rows


class Run:
    def __init__(self, out):
        self.out = out
        self.commands = []
        self.env = os.environ.copy()
        self.target = out.parent.parent / "qualification-build" / out.name
        self.target.mkdir(parents=True, exist_ok=True)
        self.env["CARGO_TARGET_DIR"] = str(self.target)
        self.env["CARGO_NET_OFFLINE"] = "true"
        out.mkdir(parents=True, exist_ok=True)
        (out / "receipt.json").unlink(missing_ok=True)

    def __call__(self, label, command, cwd=ROOT):
        command = list(map(str, command))
        print("+ " + " ".join(command), flush=True)
        result = subprocess.run(command, cwd=cwd, env=self.env, text=True, capture_output=True)
        (self.out / f"{label}.log").write_text(result.stdout + result.stderr)
        self.commands.append({"label": label, "command": command, "exit": result.returncode})
        (self.out / "commands.json").write_text(json.dumps(self.commands, indent=2) + "\n")
        if result.returncode:
            print(result.stdout + result.stderr, flush=True)
            raise SystemExit(f"Failed: {self.out / (label + '.log')}")
        return result.stdout


def package_sources(run, sdk_source):
    sdk_source, identity = verified_sdk_source(sdk_source)
    sources, hashes = {}, {}
    for source in [*(sdk_source / p for p in SDK_PATHS), ROOT / "crates/openbindings-openapi"]:
        package = tomllib.loads((source / "Cargo.toml").read_text())["package"]
        name = package["name"]
        version = package["version"]
        if not isinstance(version, str):
            version = "0.2.0-alpha.1"
        run("package-" + name, ["cargo", "package", "--locked", "--offline", "--allow-dirty", "--no-verify", "--exclude-lockfile", "--manifest-path", source / "Cargo.toml"])
        archive = run.target / "package" / f"{name}-{version}.crate"
        hashes[archive.name] = sha(archive)
        extracted = run.out / "cargo" / archive.stem
        if extracted.exists():
            shutil.rmtree(extracted)
        extract_fresh(archive, run.out / "cargo")
        normalized = tomllib.loads((extracted / "Cargo.toml").read_text())
        for section in [normalized, *normalized.get("target", {}).values()]:
            for kind in ["dependencies", "dev-dependencies", "build-dependencies"]:
                for spec in section.get(kind, {}).values():
                    if isinstance(spec, dict) and "path" in spec:
                        raise SystemExit(f"Packaged path dependency: {name}")
        sources[name] = extracted
    return sources, hashes, identity


def make_consumer(run, fixture, sources=None):
    sources = sources or {}
    consumer = run.out / "consumer"
    if consumer.exists():
        shutil.rmtree(consumer)
    shutil.copytree(fixture, consumer)
    adapter = sources.get("openbindings-openapi", ROOT / "crates/openbindings-openapi")
    (consumer / "Cargo.toml").write_text(consumer_manifest(fixture, adapter) + (patch_table(sources) if sources else ""))
    shutil.copyfile(ROOT / "Cargo.lock", consumer / "Cargo.lock")
    # Preserve the selected versions, changing only package/source identities.
    metadata = json.loads(run("consumer-metadata", ["cargo", "metadata", "--offline", "--format-version", "1"], consumer))
    identities = check_metadata(metadata, sources)
    return consumer, identities


def qualify(name):
    parser = argparse.ArgumentParser(description="Qualify the independent " + name + " consumer using pinned Git sources or extracted Cargo archives.")
    parser.add_argument("--archives", action="store_true")
    parser.add_argument("--sdk-source", type=Path, help="Clean cached SDK checkout at " + SDK_REV + "; required only for archives")
    parser.add_argument("--wasm", action="store_true")
    parser.add_argument("--tsc", type=Path, default=ROOT / "node_modules/typescript/bin/tsc")
    args = parser.parse_args()
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    run = Run(target / f"openapi-{name}-qualification" / ("archives" if args.archives else "source"))
    check_pins()
    started = input_hashes()
    sources, archives, sdk_identity = {}, {}, None
    metadata = json.loads(run("root-metadata", ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"]))
    check_metadata(metadata)
    if args.archives:
        sources, archives, sdk_identity = package_sources(run, args.sdk_source)
    fixture = ROOT / f"examples/openapi-{name}-consumer"
    consumer, identities = make_consumer(run, fixture, sources)
    run("consumer-test", ["cargo", "test", "--locked", "--offline"], consumer)
    run("consumer-lint", ["cargo", "clippy", "--locked", "--offline", "--lib", "--tests", "--", "-D", "warnings"], consumer)
    example_names = {entry.get("path"): entry["name"] for entry in tomllib.loads((consumer / "Cargo.toml").read_text()).get("example", [])}
    for example in sorted((consumer / "examples").glob("*.rs")):
        example_name = example_names.get("examples/" + example.name, example.stem)
        lints = ["-D", "warnings"]
        if example.stem == "auth_resolution":
            # The preserved stopped experiment is compiled, never executed.
            lints += ["-A", "clippy::type_complexity", "-A", "clippy::too_many_arguments"]
        run("consumer-lint-" + example.stem, ["cargo", "clippy", "--locked", "--offline", "--example", example_name, "--", *lints], consumer)
        if example.stem == "author":
            run("consumer-author", ["cargo", "run", "--locked", "--offline", "--example", "author"], consumer)
    tree = run("core-isolation", ["cargo", "tree", "--locked", "--offline", "-p", "openbindings", "--edges", "normal"], consumer)
    if "dynamic-openapi-client" in tree or "openbindings-openapi" in tree:
        raise SystemExit("OpenAPI dependency leaked into core")
    if args.wasm:
        if not args.tsc.is_file():
            raise SystemExit("Run npm ci --offline --ignore-scripts, or supply --tsc")
        run("wasm-build", ["cargo", "build", "--locked", "--offline", "--lib", "--target", "wasm32-unknown-unknown", "--features", "wasm-probe"], consumer)
        host = run.out / "host"
        host.mkdir(exist_ok=True)
        binary = "openapi_" + name.replace("-", "_") + "_consumer"
        run("wasm-bindgen", ["wasm-bindgen", "--target", "nodejs", "--out-dir", host / "wasm", run.target / f"wasm32-unknown-unknown/debug/{binary}.wasm"])
        shutil.copyfile(fixture / "wasm-probe.ts", host / "wasm-probe.ts")
        run("typescript", ["node", args.tsc.resolve(), "--strict", "--target", "ES2022", "--module", "commonjs", "--lib", "ES2022,DOM", host / "wasm-probe.ts"])
        run("wasm-host", ["node", host / "wasm-probe.js"])
    if started != input_hashes():
        raise SystemExit("Qualification inputs changed during this run; no receipt written")
    head = subprocess.run(["git", "rev-parse", "--verify", "HEAD"], cwd=ROOT, capture_output=True, text=True)
    receipt = {
        "status": "pass", "testedInputs": started, "sdkSource": sdk_identity,
        "head": head.stdout.strip() if head.returncode == 0 else None,
        "archives": archives, "wasmProbe": args.wasm, "sources": identities,
        "archiveInputs": {p.relative_to(run.out).as_posix(): sha(p) for p in sorted((run.out / "cargo").rglob("*")) if p.is_file()},
        "consumerLockSha256": sha(consumer / "Cargo.lock"), "commands": run.commands,
        "rustc": run("rustc", ["rustc", "--version"]).strip(),
    }
    (run.out / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(f"Passed; receipt: {run.out / 'receipt.json'}")


def qualify_minimal():
    parser = argparse.ArgumentParser(description="Check the minimal invocation consumer; --archives reuses the preceding JSON-path archive qualification.")
    parser.add_argument("--archives", action="store_true")
    args = parser.parse_args()
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    run = Run(target / "openapi-invocation-only-qualification" / ("archives" if args.archives else "source"))
    check_pins()
    started = input_hashes()
    sources, archive_receipt = {}, None
    if args.archives:
        qualified = target / "openapi-json-path-qualification/archives"
        archive_receipt = qualified / "receipt.json"
        if not archive_receipt.is_file():
            raise SystemExit("Run JSON-path archive qualification first")
        receipt = json.loads(archive_receipt.read_text())
        if receipt.get("status") != "pass" or receipt["testedInputs"] != started:
            raise SystemExit("Qualified inputs changed")
        for path, expected in receipt["archiveInputs"].items():
            if sha(qualified / path) != expected:
                raise SystemExit("Qualified archive inputs changed")
        if receipt["sdkSource"]["commit"] != SDK_REV:
            raise SystemExit("Unexpected SDK archive identity")
        sources = {name: Path(path) for name, path in receipt["sources"].items() if name in SDK_NAMES or name == "openbindings-openapi"}
    consumer, identities = make_consumer(run, ROOT / "examples/openapi-invocation-only", sources)
    run("consumer-check", ["cargo", "check", "--locked", "--offline"], consumer)
    run("consumer-lint", ["cargo", "clippy", "--locked", "--offline", "--", "-D", "warnings"], consumer)
    tree = run("minimal-tree", ["cargo", "tree", "--locked", "--offline", "--edges", "normal"], consumer)
    for forbidden in ["openbindings-json-schema-evaluator", "reqwest", "tokio", "wasm-bindgen"]:
        if forbidden in tree:
            raise SystemExit("Unexpected minimal consumer dependency: " + forbidden)
    if started != input_hashes():
        raise SystemExit("Qualification inputs changed")
    (run.out / "receipt.json").write_text(json.dumps({"status": "pass", "testedInputs": started, "sources": identities, "archiveReceiptSha256": sha(archive_receipt) if archive_receipt else None, "commands": run.commands}, indent=2) + "\n")
    print("Minimal invocation-only consumer passed")
