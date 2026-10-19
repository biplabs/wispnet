#!/usr/bin/env python3
"""Install the existing node, with desktop/tray only in a graphical session."""

import argparse
from pathlib import Path
import os
import filecmp
import shutil
import subprocess
import time
import hashlib
import json
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument(
    "--no-start",
    action="store_true",
    help="Install files only; do not enable, start or restart services",
)
parser.add_argument(
    "--headless",
    action="store_true",
    help="Install only the node; skip Qt, desktop launcher and tray",
)
args = parser.parse_args()
ROOT = Path(__file__).resolve().parents[1]
node_source = ROOT / "bin/wispnet"
if node_source.exists():
    # Distribution checks happen before changing any installed file or service.
    manifest = json.loads((ROOT / "manifest.json").read_text())
    for name, expected in manifest["sha256"].items():
        path = ROOT / name
        if (
            Path(name).is_absolute()
            or ".." in Path(name).parts
            or path.is_symlink()
            or hashlib.sha256(path.read_bytes()).hexdigest() != expected
        ):
            raise SystemExit(f"Bundle checksum mismatch: {name}")
else:
    node_source = ROOT / "target/debug/wispnet"
if sys.version_info < (3, 11):
    raise SystemExit("Python 3.11 or newer is required")
subprocess.run([str(node_source), "--help"], check=True, stdout=subprocess.DEVNULL)
home = Path.home()
# Upgrade the misspelled installation without moving or recreating identity/state.
# The copied configuration deliberately retains its original state_dir.
legacy_config = home / ".config/wipsnet-node/host.toml"
config = home / ".config/wispnet-node/host.toml"
if legacy_config.exists() and not config.exists():
    config.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    config.parent.chmod(0o700)
    with config.open("x") as destination_config:
        config.chmod(0o600)
        destination_config.write(legacy_config.read_text())
    for helper in ["admin-targets.json"]:
        legacy_helper = legacy_config.parent / helper
        if legacy_helper.exists():
            target_helper = config.parent / helper
            with target_helper.open("x") as output_helper:
                target_helper.chmod(0o600)
                output_helper.write(legacy_helper.read_text())
units = home / ".config/systemd/user"
units.mkdir(parents=True, exist_ok=True)
node_unit = units / "wispnet-host.service"
shutil.copy2(ROOT / "deploy/node/wispnet-host.service", node_unit)
node_binary = home / ".local/libexec/wispnet-node/wispnet"
node_binary.parent.mkdir(parents=True, exist_ok=True)
node_changed = not node_binary.exists() or not filecmp.cmp(
    node_source, node_binary, shallow=False
)
if node_changed:
    node_temporary = node_binary.with_name("wispnet.installing")
    shutil.copy2(node_source, node_temporary)
    node_temporary.chmod(0o755)
    os.replace(node_temporary, node_binary)
cli_launcher = home / ".local/bin/wispnet"
cli_launcher.parent.mkdir(parents=True, exist_ok=True)
cli_temporary = cli_launcher.with_name("wispnet.installing")
cli_temporary.unlink(missing_ok=True)
cli_temporary.symlink_to(node_binary)
os.replace(cli_temporary, cli_launcher)
# The headless command shares the existing node API and needs only Python stdlib.
api_package = node_binary.parent / "wispnet_desktop"
api_package.mkdir(exist_ok=True)
for name in ["__init__.py", "api.py", "combined.py"]:
    shutil.copy2(ROOT / "apps/node/desktop/wispnet_desktop" / name, api_package / name)
shutil.copy2(ROOT / "scripts/node_request.py", node_binary.parent / "node_request.py")
node_launcher = home / ".local/bin/wispnet-node"
node_launcher.parent.mkdir(parents=True, exist_ok=True)
node_launcher.write_text(
    "#!/usr/bin/env python3\nimport os,sys\nos.execv(sys.executable, [sys.executable, "
    + repr(str(node_binary.parent / "node_request.py"))
    + ",*sys.argv[1:]])\n"
)
node_launcher.chmod(0o755)
if not args.no_start:
    legacy_unit = units / "wipsnet-host.service"
    if legacy_unit.exists():
        subprocess.run(["systemctl", "--user", "disable", "--now", "wipsnet-host.service"], check=True)
        legacy_unit.unlink()
    subprocess.run(["systemctl", "--user", "daemon-reload"], check=True)
    subprocess.run(
        ["systemctl", "--user", "enable", "wispnet-host.service"], check=True
    )
if not args.no_start and (
    node_changed
    or subprocess.run(
        ["systemctl", "--user", "is-active", "--quiet", "wispnet-host.service"]
    ).returncode
):
    subprocess.run(
        ["systemctl", "--user", "restart", "wispnet-host.service"], check=True
    )
if not args.no_start:
    deadline = time.monotonic() + 10
    while True:
        ready = subprocess.run([str(node_launcher)], stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE, timeout=30)
        if ready.returncode == 0:
            break
        if time.monotonic() >= deadline:
            raise RuntimeError("Installed node did not become ready; inspect wispnet-host.service")
        time.sleep(0.1)
    # Remove only the obsolete executable after the replacement answers IPC.
    (home / ".local/libexec/wipsnet-node/wipsnet").unlink(missing_ok=True)
# Do not pull in Qt or start a display-dependent service on a headless host.
graphical = (
    not args.headless
    and subprocess.run(
        ["systemctl", "--user", "is-active", "--quiet", "graphical-session.target"]
    ).returncode
    == 0
)
if not graphical:
    print(
        "Installed headless node; Qt/desktop/tray skipped. Identity/configuration/grants preserved."
    )
    raise SystemExit(0)

destination = home / ".local/libexec/wispnet-desktop"
destination.mkdir(parents=True, exist_ok=True)
source = ROOT / "apps/node/desktop"
for name in ["pyproject.toml", "uv.lock", "icon.svg", "icon.png"]:
    shutil.copy2(source / name, destination / name)
shutil.copytree(
    source / "wispnet_desktop",
    destination / "wispnet_desktop",
    dirs_exist_ok=True,
    ignore=shutil.ignore_patterns("__pycache__"),
)
build_info = destination / "wispnet_desktop/build_info.py"
if not (source / "wispnet_desktop/build_info.py").exists():
    label = subprocess.check_output(["python3", str(ROOT / "scripts/build_identity.py")], text=True).strip()
    build_info.write_text("DISPLAY_VERSION = " + repr(label) + "\n")
subprocess.run(["uv", "sync", "--locked", "--project", str(destination)], check=True)
binpath = home / ".local/bin/wispnet-desktop"
binpath.parent.mkdir(parents=True, exist_ok=True)
binpath.write_text(
    "#!/usr/bin/env python3\nimport os,sys\nos.execv("
    + repr(str(destination / ".venv/bin/wispnet-desktop"))
    + ', ["wispnet-desktop",*sys.argv[1:]])\n'
)
binpath.chmod(0o755)
applications = home / ".local/share/applications"
applications.mkdir(parents=True, exist_ok=True)
(applications / "wispnet.desktop").write_text(
    '[Desktop Entry]\nType=Application\nName=Wispnet\nComment=Your services and connected devices\nExec="'
    + str(binpath)
    + '"\nIcon='
    + str(destination / "icon.png")
    + "\nCategories=Network;\nTerminal=false\n"
)
shutil.copy2(
    ROOT / "deploy/node/wispnet-desktop.service", units / "wispnet-desktop.service"
)
if args.no_start:
    print("Installed desktop files; services were not enabled or started.")
    raise SystemExit(0)
subprocess.run(["systemctl", "--user", "daemon-reload"], check=True)
subprocess.run(["systemctl", "--user", "enable", "wispnet-desktop.service"], check=True)
subprocess.run(
    ["systemctl", "--user", "restart", "wispnet-desktop.service"], check=True
)
runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
deadline = time.monotonic() + 10
while time.monotonic() < deadline:
    pid = subprocess.check_output(
        [
            "systemctl",
            "--user",
            "show",
            "wispnet-desktop.service",
            "-p",
            "MainPID",
            "--value",
        ],
        text=True,
    ).strip()
    try:
        if (
            pid != "0"
            and (runtime / "wispnet-desktop.lock").read_text().splitlines()[0] == pid
            and (runtime / "wispnet-desktop.ui").exists()
        ):
            break
    except OSError:
        pass
    time.sleep(0.1)
else:
    raise RuntimeError(
        "Desktop tray did not become ready; inspect wispnet-desktop.service"
    )
print(
    "Installed launcher and session tray service; identity/configuration/grants preserved."
)
