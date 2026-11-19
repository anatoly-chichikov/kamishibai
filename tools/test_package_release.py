"""Exercise release packaging against a local Cargo graph and real archives."""

import shutil
import subprocess
from pathlib import Path

import pytest
from package_release import package_release


def crate(root: Path, name: str, dependencies: str) -> None:
    """Write a local crate whose dependencies Cargo can resolve offline."""
    (root / "src").mkdir(parents=True)
    (root / "src" / "lib.rs").touch()
    (root / "Cargo.toml").write_text(
        f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2024"\nlicense="MIT"\n{dependencies}',
        encoding="utf-8",
    )


@pytest.mark.parametrize(
    ("target", "extension", "executable"),
    [
        ("aarch64-apple-darwin", "tar.gz", "kamishibai"),
        ("x86_64-pc-windows-msvc", "zip", "kamishibai.exe"),
    ],
)
def test_release_archives_cannot_lose_an_embedded_license_or_notice(
    tmp_path: Path, target: str, extension: str, executable: str
) -> None:
    """Verify all distributed members and a transitive notice in both archive formats."""
    crate(
        tmp_path,
        "kamishibai",
        '[dependencies]\nreader={path="reader"}\n[dev-dependencies]\nfixture={path="fixture"}',
    )
    crate(tmp_path / "reader", "reader", "")
    crate(tmp_path / "fixture", "fixture", "")
    (tmp_path / "reader" / "algorithms").mkdir()
    (tmp_path / "reader" / "LICENSE").write_text(
        "upstream permission", encoding="utf-8"
    )
    (tmp_path / "reader" / "algorithms" / "NOTICE").write_text(
        "original algorithm credit", encoding="utf-8"
    )
    resources = tmp_path / "reader" / "src" / "res"
    resources.mkdir()
    (resources / "CoatedFOGRA39.icc.LICENSE.txt").write_text(
        "profile distribution terms", encoding="utf-8"
    )
    (resources / "data.copying.txt").write_text(
        "data distribution terms", encoding="utf-8"
    )
    (resources / "font.NOTICE.txt").write_text("font credit", encoding="utf-8")
    (tmp_path / "reader" / "PATENTS").write_bytes(
        "grant to “Affiliate”".encode("windows-1252")
    )
    for member in (executable, "llms.txt", "LICENSE", "NOTICE"):
        (tmp_path / member).write_text(member, encoding="utf-8")
    subprocess.run(
        ["cargo", "generate-lockfile", "--offline"], cwd=tmp_path, check=True
    )
    archive = tmp_path / f"release.{extension}"
    package_release(tmp_path, target, tmp_path / executable, archive)
    restored = tmp_path / "restored"
    shutil.unpack_archive(archive, restored)
    bundle = (restored / "THIRD-PARTY.txt").read_text(encoding="utf-8")
    members = {
        p.name: p.read_text(encoding="utf-8")
        for p in restored.iterdir()
        if p.name != "THIRD-PARTY.txt"
    }
    assert (
        members
        == {name: name for name in (executable, "llms.txt", "LICENSE", "NOTICE")}
        and "reader 0.1.0 / LICENSE\n\nupstream permission" in bundle
        and "reader 0.1.0 / algorithms/NOTICE\n\noriginal algorithm credit" in bundle
        and "reader 0.1.0 / PATENTS\n\ngrant to “Affiliate”" in bundle
        and "reader 0.1.0 / src/res/CoatedFOGRA39.icc.LICENSE.txt\n\nprofile distribution terms"
        in bundle
        and "reader 0.1.0 / src/res/data.copying.txt\n\ndata distribution terms"
        in bundle
        and "reader 0.1.0 / src/res/font.NOTICE.txt\n\nfont credit" in bundle
        and "fixture 0.1.0" not in bundle
    ), "the archive lost a release member or distributed the wrong component notices"
