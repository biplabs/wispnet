#!/usr/bin/env python3
"""Build a checkout-independent Linux x86_64 distribution and SHA-256 sidecar."""

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def package(binary, output):
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise ValueError("Only native Linux x86_64 bundles are supported")
    subprocess.run([str(binary), "--help"], check=True, stdout=subprocess.DEVNULL)
    version_info = subprocess.check_output(
        ["readelf", "--version-info", str(binary)], text=True
    )
    glibc = max(
        re.findall(r"GLIBC_(\d+\.\d+)", version_info),
        key=lambda v: tuple(map(int, v.split("."))),
    )
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    epoch = int(
        os.environ.get("SOURCE_DATE_EPOCH")
        or subprocess.check_output(
            ["git", "show", "-s", "--format=%ct", "HEAD"], cwd=ROOT, text=True
        )
    )
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        tree = Path(temporary) / "wispnet-linux-x86_64"
        tree.mkdir()
        files = [
            "scripts/install_node_desktop.py",
            "scripts/node_request.py",
            "deploy/node/wispnet-host.service",
            "deploy/node/wispnet-desktop.service",
            "config/node/host.toml",
            "docs/linux-distribution.md",
            "docs/linux-desktop.md",
            "docs/per-host-control.md",
        ]
        desktop = ROOT / "apps/node/desktop"
        files.extend(
            str((desktop / name).relative_to(ROOT))
            for name in ["pyproject.toml", "uv.lock", "icon.png", "icon.svg"]
        )
        files.extend(
            str(p.relative_to(ROOT)) for p in (desktop / "wispnet_desktop").glob("*.py")
        )
        files.append("apps/node/desktop/wispnet_desktop/icon.png")
        files.extend(str(p.relative_to(ROOT)) for p in (ROOT / "docs").rglob("*.md"))
        files.append("README.md")
        for name in sorted(set(files)):
            target = tree / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
        build_label = subprocess.check_output(["python3", str(ROOT / "scripts/build_identity.py")], text=True).strip()
        (tree / "apps/node/desktop/wispnet_desktop/build_info.py").write_text("DISPLAY_VERSION = " + repr(build_label) + "\n")
        (tree / "bin").mkdir()
        shutil.copy2(binary, tree / "bin/wispnet")
        subprocess.run(
            ["strip", "--strip-debug", str(tree / "bin/wispnet")], check=True
        )
        hashes = {
            str(p.relative_to(tree)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(tree.rglob("*"))
            if p.is_file()
        }
        (tree / "manifest.json").write_text(
            json.dumps(
                {
                    "format": 1,
                    "source_commit": revision,
                    "source_dirty": bool(
                        subprocess.check_output(
                            [
                                "git",
                                "status",
                                "--porcelain",
                                "--untracked-files=normal",
                            ],
                            cwd=ROOT,
                        )
                    ),
                    "platform": "linux-x86_64",
                    "minimum_glibc": glibc,
                    "python": ">=3.11",
                    "sha256": hashes,
                },
                indent=2,
            )
            + "\n"
        )
        with (
            output.open("wb") as raw,
            gzip.GzipFile(
                filename="", mode="wb", fileobj=raw, mtime=epoch
            ) as compressed,
            tarfile.open(fileobj=compressed, mode="w") as archive,
        ):
            for path in [tree, *sorted(tree.rglob("*"))]:
                info = archive.gettarinfo(
                    str(path), arcname=str(path.relative_to(tree.parent))
                )
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = epoch
                if path.is_file():
                    with path.open("rb") as source:
                        archive.addfile(info, source)
                else:
                    archive.addfile(info)
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_name(output.name + ".sha256").write_text(f"{digest}  {output.name}\n")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary", type=Path, help="Use an explicitly prebuilt native binary"
    )
    parser.add_argument(
        "--output", type=Path, default=ROOT / "target/dist/wispnet-linux-x86_64.tar.gz"
    )
    args = parser.parse_args()
    if args.binary is None:
        subprocess.run(
            ["cargo", "build", "--release", "--locked", "-p", "wispnet-cli"],
            cwd=ROOT,
            check=True,
        )
    binary = (args.binary or ROOT / "target/release/wispnet").resolve()
    print(package(binary, args.output))


if __name__ == "__main__":
    main()
