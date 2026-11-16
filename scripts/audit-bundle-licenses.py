"""Report license files for each locked package in a verified source bundle."""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import tarfile
import tomllib


def identity(package):
    return package["name"], package["version"]


def files_in(directory, prefixes):
    return sorted(
        path for path in directory.iterdir()
        if path.name.lower().startswith(prefixes)
        and not path.is_symlink() and path.is_file() and path.stat().st_size > 0
    )


def sha256(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def evidence(bundle, checksums, path):
    name = path.relative_to(bundle).as_posix()
    if path.resolve(strict=True) != path or not path.is_file():
        raise ValueError(f"expected regular bundle file: {name}")
    digest = sha256(path)
    if checksums.get(name) != digest:
        raise ValueError(f"file missing from bundle or checksum differs: {name}")
    return {"path": name, "sha256": digest}


def registry_evidence(bundle, checksums, directory, locked, crate_cache):
    manifest = directory / ".cargo-checksum.json"
    evidence(bundle, checksums, manifest)
    vendor = json.loads(manifest.read_text())
    if vendor["package"] != locked["checksum"]:
        raise ValueError(f"vendor checksum differs from Cargo.lock: {directory.name}")
    expected = vendor["files"]
    bundled = {
        Path(name).relative_to(directory.relative_to(bundle)).as_posix()
        for name in checksums
        if Path(name).is_relative_to(directory.relative_to(bundle))
        and name != manifest.relative_to(bundle).as_posix()
    }
    if bundled != expected.keys():
        raise ValueError(f"vendored file set differs: {directory.name}")
    for name, checksum in expected.items():
        if evidence(bundle, checksums, directory / name)["sha256"] != checksum:
            raise ValueError(f"vendored file checksum differs: {directory.name}/{name}")

    result = {"archive_sha256": locked["checksum"], "vendored_files": len(expected)}
    if crate_cache is not None:
        crate = f"{locked['name']}-{locked['version']}"
        archive = crate_cache / f"{crate}.crate"
        if sha256(archive) != locked["checksum"]:
            raise ValueError(f"cached archive checksum differs: {crate}")
        remaining = set(expected)
        with tarfile.open(archive, "r:gz") as package:
            for member in package:
                if not member.name.startswith(crate + "/"):
                    continue
                name = member.name.removeprefix(crate + "/")
                if name not in expected:
                    continue
                if name not in remaining or not member.isfile():
                    raise ValueError(f"ambiguous archive member: {crate}/{name}")
                with package.extractfile(member) as file:
                    checksum = hashlib.file_digest(file, "sha256").hexdigest()
                if checksum != expected[name]:
                    raise ValueError(f"archive member differs: {crate}/{name}")
                remaining.remove(name)
        if remaining:
            raise ValueError(f"vendored files missing from archive: {crate}")
        result["archive_verified"] = True
    return result


def audit(bundle, crate_cache=None):
    source = bundle / "source"
    checksums = json.loads((bundle / "checksums.json").read_text())
    inventory = bundle / "inventory.json"
    lockfile = source / "Cargo.lock"
    for path in (inventory, lockfile):
        evidence(bundle, checksums, path)
    packages = json.loads(inventory.read_text())["packages"]
    locked = tomllib.loads(lockfile.read_text())["package"]
    if Counter(map(identity, packages)) != Counter(map(identity, locked)):
        raise ValueError("inventory and lockfile package identities differ")
    locked_by_identity = {identity(package): package for package in locked}
    if len(locked_by_identity) != len(locked):
        raise ValueError("lockfile has ambiguous package identities")

    results = []
    for package in packages:
        manifest = source / package["manifest"]
        if not manifest.resolve(strict=True).is_relative_to(source):
            raise ValueError("package manifest is outside the source bundle")
        evidence(bundle, checksums, manifest)
        roots = [manifest.parent]
        if package["source"] == "workspace" and manifest.parent != source:
            roots.append(source)
        licenses = sorted(set(path for root in roots for path in
                              files_in(root, ("license", "copying", "unlicense"))))
        notices = sorted(set(path for root in roots for path in
                             files_in(root, ("notice", "copyright"))))
        result = {
            **{key: package[key] for key in ("name", "version", "source", "declared_license")},
            "license_files": [evidence(bundle, checksums, path) for path in licenses],
            "notice_files": [evidence(bundle, checksums, path) for path in notices],
        }
        record = locked_by_identity[identity(package)]
        if package["source"] == "crates.io":
            if record.get("source") != "registry+https://github.com/rust-lang/crates.io-index":
                raise ValueError(f"registry source differs: {package['name']}")
            result["registry"] = registry_evidence(
                bundle, checksums, manifest.parent, record, crate_cache,
            )
        elif record.get("source") is not None:
            raise ValueError(f"unexpected nonlocal source: {package['name']}")
        results.append(result)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--crate-cache", type=Path,
                        help="registry directory containing the original .crate archives")
    args = parser.parse_args()
    try:
        packages = audit(args.bundle.resolve(strict=True), args.crate_cache)
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        parser.error(str(error))
    missing = [p["name"] for p in packages if not p["license_files"]]
    print(json.dumps({"packages": packages, "missing_license_files": missing}, indent=2))
    return 1 if missing else 0


if __name__ == "__main__":
    raise SystemExit(main())
