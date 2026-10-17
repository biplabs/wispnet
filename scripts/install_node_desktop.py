#!/usr/bin/env python3
"""Install the user-local desktop/tray without changing node identity/configuration."""

from pathlib import Path
import os
import filecmp
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
home = Path.home()
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
subprocess.run(["uv", "sync", "--locked", "--project", str(destination)], check=True)
binpath = home / ".local/bin/wispnet-desktop"
binpath.parent.mkdir(parents=True, exist_ok=True)
# Exec uses argv, never a shell-interpolated invitation or service name.
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
units = home / ".config/systemd/user"
units.mkdir(parents=True, exist_ok=True)
node_unit = units / "wipsnet-host.service"
if not node_unit.exists():
    shutil.copy2(ROOT / "deploy/node/wipsnet-host.service", node_unit)
node_binary = home / ".local/libexec/wipsnet-node/wipsnet"
node_binary.parent.mkdir(parents=True, exist_ok=True)
node_changed = not node_binary.exists() or not filecmp.cmp(
    ROOT / "target/debug/wipsnet", node_binary, shallow=False
)
if node_changed:
    node_temporary = node_binary.with_name("wipsnet.installing")
    shutil.copy2(ROOT / "target/debug/wipsnet", node_temporary)
    node_temporary.chmod(0o755)
    os.replace(node_temporary, node_binary)
shutil.copy2(
    ROOT / "deploy/node/wispnet-desktop.service", units / "wispnet-desktop.service"
)
subprocess.run(["systemctl", "--user", "daemon-reload"], check=True)
if (
    node_changed
    or subprocess.run(
        ["systemctl", "--user", "is-active", "--quiet", "wipsnet-host.service"]
    ).returncode
):
    subprocess.run(
        ["systemctl", "--user", "restart", "wipsnet-host.service"], check=True
    )
subprocess.run(["systemctl", "--user", "enable", "wispnet-desktop.service"], check=True)
subprocess.run(
    ["systemctl", "--user", "restart", "wispnet-desktop.service"], check=True
)
runtime=Path(os.environ.get('XDG_RUNTIME_DIR',f'/run/user/{os.getuid()}'))
deadline=time.monotonic()+10
while time.monotonic()<deadline:
    pid=subprocess.check_output(['systemctl','--user','show','wispnet-desktop.service','-p','MainPID','--value'],text=True).strip()
    try:
        if pid!='0' and (runtime/'wispnet-desktop.lock').read_text().splitlines()[0]==pid and (runtime/'wispnet-desktop.ui').exists():break
    except OSError:pass
    time.sleep(.1)
else:raise RuntimeError('Desktop tray did not become ready; inspect wispnet-desktop.service')
print(
    "Installed launcher and session tray service; identity/configuration/grants preserved."
)
