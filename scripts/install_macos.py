#!/usr/bin/env python3
"""Install the Flutter macOS app and user launch agents from a checkout or bundle."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
LABEL = "app.wispnet.node"
DESKTOP_LABEL = "app.wispnet.desktop"


def distribution():
    """Verify every packaged payload before touching configuration or installed files."""
    if not (ROOT / "manifest.json").exists():
        if (ROOT / "bin/wispnet").exists():
            raise ValueError("Distribution manifest is missing")
        return None
    manifest = json.loads((ROOT / "manifest.json").read_text())
    arch = {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64"}.get(platform.machine())
    if (manifest.get("format") != 1 or manifest.get("platform") != f"macos-{arch}"
        or manifest.get("desktop") != "flutter" or manifest.get("license") != "Apache-2.0"):
        raise ValueError("Unsupported distribution platform or format")
    if (not re.fullmatch(r"[0-9]+", str(manifest.get("source_count", "")))
        or not re.fullmatch(r"[0-9]+(?:\.[0-9]+){1,3}", manifest.get("version", ""))
        or not isinstance(manifest.get("build_identity"), str)):
        raise ValueError("Invalid distribution build metadata")
    hashes = manifest["sha256"]
    symlinks = manifest["symlinks"]
    required = {"bin/wispnet", "Wispnet.app/Contents/MacOS/Wispnet", "scripts/install_macos.py"}
    if not required.issubset(hashes) or set(hashes) & set(symlinks):
        raise ValueError("Distribution is missing required payloads")
    actual = set()
    for path in ROOT.rglob("*"):
        if path.is_symlink() or path.is_file():
            actual.add(str(path.relative_to(ROOT)))
        elif not path.is_dir():
            raise ValueError(f"Unsupported distribution payload: {path.name}")
    if actual != set(hashes) | set(symlinks) | {"manifest.json"}:
        raise ValueError("Distribution payload does not match its manifest")
    for name, expected in {**hashes, **symlinks}.items():
        relative = Path(name)
        path = ROOT / relative
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"Invalid distribution path: {name}")
        if name in symlinks:
            if (not path.is_symlink() or os.readlink(path) != expected
                or Path(expected).is_absolute() or not path.exists()
                or not path.resolve().is_relative_to(ROOT.resolve())):
                raise ValueError(f"Bundle checksum mismatch: {name}")
        elif path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f"Bundle checksum mismatch: {name}")
    return manifest


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def write_plist(path, value):
    temporary = path.with_suffix(".installing")
    with temporary.open("wb") as output:
        os.chmod(temporary, 0o600)
        plistlib.dump(value, output)
    os.replace(temporary, path)


def bootstrap(domain, plist):
    # launchd may briefly reject bootstrap while the previous job is unloading.
    # Retry the same job for a bounded interval, preserving its diagnostic.
    deadline = time.monotonic() + 10
    while True:
        result = subprocess.run(["launchctl", "bootstrap", domain, str(plist)],
                                capture_output=True, text=True, timeout=30)
        if result.returncode == 0:
            return
        if time.monotonic() >= deadline:
            raise RuntimeError(result.stderr.strip() or f"Could not start {plist.name}")
        time.sleep(0.2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--init", action="store_true", help="Explicitly initialize a new Mac identity if no configuration exists")
    parser.add_argument("--no-start", action="store_true", help="Install files without changing running jobs")
    parser.add_argument("--skip-build", action="store_true", help="Use the already built release app and debug node")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("This installer requires macOS")
    if sys.version_info < (3, 11):
        parser.error("Python 3.11 or newer is required")
    try:
        manifest = distribution()
    except (ValueError, KeyError, OSError) as error:
        parser.error(str(error))
    home = Path.home()
    config = home / ".config/wispnet-node/host.toml"
    state = home / ".local/state/wispnet-node"
    if not config.exists() and not args.init:
        parser.error("No existing host configuration. Use --init to explicitly create this Mac’s independent identity.")
    if manifest is None and not args.skip_build:
        run("cargo", "build", "--workspace", "--locked", cwd=ROOT)
        run(ROOT / "scripts/build_flutter_desktop.sh")
    node_source = ROOT / ("bin/wispnet" if manifest else "target/debug/wispnet")
    app_source = ROOT / ("Wispnet.app" if manifest else "apps/flutter/build/macos/Build/Products/Release/Wispnet.app")
    run(node_source, "--help", stdout=subprocess.DEVNULL)
    run("codesign", "--verify", "--deep", "--strict", app_source)
    if manifest:
        version = manifest["build_identity"]
        build_number = manifest["source_count"]
        app_version = manifest["version"]
    else:
        version = subprocess.check_output([sys.executable, str(ROOT / "scripts/build_identity.py")], text=True).strip()
        build_number = subprocess.check_output(["git", "rev-list", "--count", "HEAD"], cwd=ROOT, text=True).strip()
        app_version = "0.1.0"
    if not config.exists():
        # Never recover missing authority by silently creating a new identity.
        if state.exists():
            parser.error("State exists without configuration; recover its configuration before installing")
        config.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        content = (f'schema_version=2\ndevice_name={json.dumps(os.uname().nodename.split(".")[0])}\n'
                   f'state_dir={json.dumps(str(state))}\nlog_level="info"\n'
                   '[transport]\nbind_addr="0.0.0.0:0"\nrelay_url="https://relay.wispnet.app"\nrelay_only=false\n')
        with config.open("x") as output:
            os.chmod(config, 0o600)
            output.write(content)
        run(node_source, "host-init", "--config", config)
    state = Path(tomllib.loads(config.read_text())["state_dir"])
    if not state.is_absolute():
        parser.error("Existing node state directory must be absolute")
    # Validate authority before replacing executable files or restarting services.
    run(node_source, "host-status", "--config", config, stdout=subprocess.DEVNULL)
    node_binary = home / ".local/libexec/wispnet-node/wispnet"
    node_binary.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary_node = node_binary.with_suffix(".installing")
    shutil.copy2(node_source, temporary_node)
    temporary_node.chmod(0o755)
    os.replace(temporary_node, node_binary)
    bin_dir = home / ".local/bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    cli = bin_dir / "wispnet"
    temporary_cli = cli.with_suffix(".installing")
    temporary_cli.unlink(missing_ok=True)
    temporary_cli.symlink_to(node_binary)
    os.replace(temporary_cli, cli)

    app = home / "Applications/Wispnet.app"
    staged = app.with_name("Wispnet.installing.app")
    if staged.exists():
        shutil.rmtree(staged)
    shutil.copytree(app_source, staged, symlinks=True)
    if app.exists():
        old = home / "Library/Application Support/Wispnet/Backups/Wispnet.previous.app"
        old.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        if old.exists():
            shutil.rmtree(old)
        os.replace(app, old)
    os.replace(staged, app)
    contents = app / "Contents"
    resources = contents / "Resources"
    executable = contents / "MacOS/Wispnet"
    # Qt-free helper retains the existing durable owner-operation journals/cache.
    shutil.copy2(
        ROOT / "apps/flutter/desktop/desktop_data.py", executable.parent / "desktop_data.py"
    )
    helper_package = executable.parent / "wispnet_desktop"
    helper_package.mkdir(exist_ok=True)
    for name in ["__init__.py", "api.py", "combined.py", "catalog_cache.py"]:
        shutil.copy2(
            ROOT / "apps/node/desktop/wispnet_desktop" / name, helper_package / name
        )
    (executable.parent / "python-path").write_text(str(Path(sys.executable).resolve()))
    # Provide the same owner automation endpoint when this Mac is an SSH target.
    shutil.copytree(
        helper_package, node_binary.parent / "wispnet_desktop", dirs_exist_ok=True
    )
    shutil.copy2(
        ROOT / "scripts/node_request.py", node_binary.parent / "node_request.py"
    )
    shutil.copy2(ROOT / "scripts/mcp_headers.py", node_binary.parent / "mcp_headers.py")
    owner_command = home / ".local/bin/wispnet-node"
    owner_command.write_text(
        "#!"
        + sys.executable
        + "\nimport os,sys\nos.execv(sys.executable, [sys.executable, "
        + repr(str(node_binary.parent / "node_request.py"))
        + ",*sys.argv[1:]])\n"
    )
    owner_command.chmod(0o755)
    # macOS icon resources, generated from the project's existing artwork.
    with tempfile.TemporaryDirectory(prefix="wispnet-icons-") as directory:
        iconset = Path(directory) / "Wispnet.iconset"
        iconset.mkdir()
        for size in [16, 32, 128, 256, 512]:
            for scale in [1, 2]:
                suffix = "@2x" if scale == 2 else ""
                run("sips", "-z", size * scale, size * scale, ROOT / "apps/node/desktop/icon.png", "--out",
                    iconset / f"icon_{size}x{size}{suffix}.png", stdout=subprocess.DEVNULL)
        run("iconutil", "-c", "icns", iconset, "-o", resources / "Wispnet.icns")
    with (contents / "Info.plist").open("rb") as source:
        app_info = plistlib.load(source)
    write_plist(contents / "Info.plist", {
        **app_info,
        "CFBundleIdentifier": "app.wispnet.desktop", "CFBundleName": "Wispnet",
        "CFBundleDisplayName": "Wispnet", "CFBundleExecutable": "Wispnet",
        "CFBundlePackageType": "APPL", "CFBundleShortVersionString": app_version,
        "CFBundleVersion": build_number, "WispnetBuildIdentity": version, "CFBundleIconFile": "Wispnet.icns",
        "LSMinimumSystemVersion": "13.0", "NSHighResolutionCapable": True,
        "WispnetStateDirectory": str(state),
    })
    run("codesign", "--force", "--deep", "--sign", "-", app)
    # Only the installed bundle should be registered as a launchable app.
    register = Path("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister")
    # Unregistering an already unregistered development build can return nonzero.
    subprocess.run([str(register), "-u", str(app_source)], capture_output=True)
    run(register, "-f", app)
    agents = home / "Library/LaunchAgents"
    agents.mkdir(parents=True, exist_ok=True)
    logs = home / "Library/Logs/Wispnet"
    logs.mkdir(parents=True, exist_ok=True, mode=0o700)
    logs.chmod(0o700)
    for name in ["node.log", "desktop.log"]:
        path = logs / name
        path.touch(mode=0o600, exist_ok=True)
        path.chmod(0o600)
    common = {"RunAtLoad": True, "KeepAlive": {"SuccessfulExit": False}, "ThrottleInterval": 3, "Umask": 0o077}
    write_plist(agents / f"{LABEL}.plist", {
        **common, "Label": LABEL,
        "ProgramArguments": [str(node_binary), "agent", "--config", str(config)],
        "StandardOutPath": str(logs / "node.log"), "StandardErrorPath": str(logs / "node.log"),
    })
    write_plist(agents / f"{DESKTOP_LABEL}.plist", {
        **common, "Label": DESKTOP_LABEL,
        "ProgramArguments": [str(executable), "--background"],
        "LimitLoadToSessionType": "Aqua",
        "StandardOutPath": str(logs / "desktop.log"), "StandardErrorPath": str(logs / "desktop.log"),
    })
    if args.no_start:
        print(f"Installed {app}; launch agents were not started.")
        return
    domain = f"gui/{os.getuid()}"
    for label in [DESKTOP_LABEL, LABEL]:
        subprocess.run(["launchctl", "bootout", domain + "/" + label], capture_output=True)
    bootstrap(domain, agents / f"{LABEL}.plist")
    deadline = time.monotonic() + 15
    while True:
        result = subprocess.run([str(owner_command)], text=True, capture_output=True, timeout=30)
        if result.returncode == 0:
            break
        if time.monotonic() >= deadline:
            raise RuntimeError(f"Installed node did not become ready; inspect {logs / 'node.log'}")
        time.sleep(0.1)
    bootstrap(domain, agents / f"{DESKTOP_LABEL}.plist")
    run("open", app)
    print(f"Installed and opened {app}. Menu bar and node start at login. Identity and grants preserved.")


if __name__ == "__main__":
    main()
