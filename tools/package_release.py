"""Package one release binary with the notices for its resolved Cargo target."""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tarfile
import zipfile
from collections import defaultdict
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import TypedDict


class Package(TypedDict):
    """Describe the published license and local source of one dependency."""

    name: str
    version: str
    manifest_path: str
    license_file: str | None
    license: str | None


class Metadata(TypedDict):
    """Read package locations from Cargo's version-one metadata."""

    packages: list[Package]


def cargo(root: Path, *arguments: str) -> str:
    """Read the locked dependency graph from the release checkout."""
    return subprocess.check_output(
        ["cargo", *arguments, "--locked"], cwd=root, text=True
    )


def licenses(package: Package) -> list[Path]:
    """Find the crate's declared license and notices, including vendored components."""
    source = Path(package["manifest_path"]).parent
    pattern = re.compile(
        r"license|copying|notice|^(?:licences?|copyright|ofl|patents)(?:$|[-._])",
        re.IGNORECASE,
    )
    files = {p for p in source.rglob("*") if pattern.search(p.name) and p.is_file()}
    if declared := package["license_file"]:
        files.add(source / declared)
    return sorted(files)


def license_text(path: Path) -> str:
    """Preserve the published text of UTF-8 and Windows-1252 notices."""
    try:
        return path.read_text(encoding="utf-8-sig").strip()
    except UnicodeDecodeError:
        return path.read_text(encoding="windows-1252").strip()


def notices(root: Path, target: str) -> str:
    """Collect the original notices for every normal dependency in the target build."""
    metadata: Metadata = json.loads(
        cargo(root, "metadata", "--format-version", "1", "--filter-platform", target)
    )
    packages = {(p["name"], "v" + p["version"]): p for p in metadata["packages"]}
    tree = cargo(
        root,
        "tree",
        "-p",
        "kamishibai",
        "--target",
        target,
        "--edges",
        "normal",
        "--prefix",
        "none",
        "--format",
        "{p}",
    )
    selected = {tuple(line.split()[:2]) for line in tree.splitlines()}
    supplemental = (root / "NOTICE").read_text(encoding="utf-8").splitlines()
    texts: defaultdict[str, list[str]] = defaultdict(list)
    inventory: list[str] = []
    for name, version in sorted(selected):
        package = packages[name, version]
        source = Path(package["manifest_path"]).parent
        if source == root:
            continue
        label = f"{name} {package['version']}"
        files = licenses(package)
        if not files and label not in supplemental:
            raise ValueError(
                f"No license file or supplemental NOTICE entry for {label}"
            )
        inventory.append(
            f"{label}  {package['license'] or 'see license file'}"
            + ("" if files else "  (see NOTICE)")
        )
        for path in files:
            texts[license_text(path)].append(
                f"{label} / {path.relative_to(source).as_posix()}"
            )
    sections = ["Cargo dependencies for " + target, "\n".join(inventory)]
    sections.extend("\n".join(paths) + "\n\n" + text for text, paths in texts.items())
    return "\n\n".join(sections) + "\n"


def package_release(root: Path, target: str, binary: Path, archive: Path) -> None:
    """Write a tar.gz or ZIP with the binary, its contract and all license materials."""
    with TemporaryDirectory(dir=archive.parent) as directory:
        staging = Path(directory)
        bundled = staging / "THIRD-PARTY.txt"
        bundled.write_text(notices(root, target), encoding="utf-8")
        files = (binary, root / "llms.txt", root / "LICENSE", root / "NOTICE", bundled)
        output = staging / archive.name
        if archive.name.endswith(".tar.gz"):
            with tarfile.open(output, "w:gz") as tar:
                for file in files:
                    tar.add(file, arcname=file.name)
        elif archive.suffix == ".zip":
            with zipfile.ZipFile(
                output, "w", compression=zipfile.ZIP_DEFLATED
            ) as zipped:
                for file in files:
                    zipped.write(file, arcname=file.name)
        else:
            raise ValueError(f"Unknown release archive format {archive.name}")
        output.replace(archive)


if __name__ == "__main__":
    checkout = Path(__file__).resolve().parents[1]
    platform, destination = sys.argv[1:]
    executable = "kamishibai.exe" if "windows" in platform else "kamishibai"
    package_release(
        checkout,
        platform,
        checkout / "target" / "release" / executable,
        Path(destination),
    )
