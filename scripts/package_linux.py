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


def package(binary, output, desktop_binary=None):
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise ValueError("Only native Linux x86_64 bundles are supported")
    subprocess.run([str(binary), "--help"], check=True, stdout=subprocess.DEVNULL)
    versions = []
    for executable in [
        binary,
        *([desktop_binary] if desktop_binary is not None else []),
    ]:
        version_info = subprocess.check_output(
            ["readelf", "--version-info", str(executable)], text=True
        )
        versions.extend(re.findall(r"GLIBC_(\d+\.\d+)", version_info))
    glibc = max(versions, key=lambda v: tuple(map(int, v.split("."))))
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
            "apps/macos/desktop_data.py",
            "deploy/node/wispnet-host.service",
            "deploy/node/wispnet-desktop.service",
            "config/node/host.toml",
            "docs/linux-distribution.md",
            "docs/linux-desktop.md",
            "docs/per-host-control.md",
        ]
        desktop = ROOT / "apps/node/desktop"
        files.extend(
            str((desktop / name).relative_to(ROOT)) for name in ["icon.png", "icon.svg"]
        )
        files.extend(
            str((desktop / "wispnet_desktop" / name).relative_to(ROOT))
            for name in ["__init__.py", "api.py", "combined.py", "catalog_cache.py"]
        )
        files.extend(str(p.relative_to(ROOT)) for p in (ROOT / "docs").rglob("*.md"))
        files.append("README.md")
        for name in sorted(set(files)):
            target = tree / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
        (tree / "bin").mkdir()
        shutil.copy2(binary, tree / "bin/wispnet")
        if desktop_binary is not None:
            shutil.copy2(desktop_binary, tree / "bin/Wispnet")
        for executable in (tree / "bin").iterdir():
            subprocess.run(["strip", "--strip-debug", str(executable)], check=True)
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
                    "desktop": "tauri" if desktop_binary is not None else "headless",
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
    parser.add_argument(
        "--headless", action="store_true", help="Exclude the Tauri desktop"
    )
    parser.add_argument(
        "--desktop-binary", type=Path, help="Use a prebuilt Tauri binary"
    )
    args = parser.parse_args()
    if args.binary is None:
        subprocess.run(
            ["cargo", "build", "--release", "--locked", "-p", "wispnet-cli"],
            cwd=ROOT,
            check=True,
        )
    binary = (args.binary or ROOT / "target/release/wispnet").resolve()
    desktop_binary = None
    if not args.headless:
        desktop_binary = args.desktop_binary
        if desktop_binary is None:
            subprocess.run(
                [
                    "cargo",
                    "build",
                    "--release",
                    "--locked",
                    "--manifest-path",
                    str(ROOT / "apps/macos/src-tauri/Cargo.toml"),
                ],
                check=True,
            )
            desktop_binary = ROOT / "apps/macos/src-tauri/target/release/Wispnet"
    print(package(binary, args.output, desktop_binary))


if __name__ == "__main__":
    main()
