#!/usr/bin/env python3
"""Uninstall this user's Wispnet programs, preserving all configuration and identity data."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def uninstall(home, uid):
    units = home / '.config/systemd/user'
    for name in ('wispnet-desktop.service', 'wispnet-host.service', 'wipsnet-host.service'):
        unit = units / name
        if unit.exists() or unit.is_symlink():
            subprocess.run(['systemctl', '--user', 'disable', '--now', name], check=True)
    if Path('/etc/wispnet-firewall.json').exists() or Path('/etc/systemd/system/wispnet-firewall.service').exists():
        subprocess.run(['sudo', sys.executable, str(ROOT / 'scripts/uninstall_firewall_helper.py'), '--uid', str(uid)], check=True)
    # Do not remove launchers/binaries if stopping services or privileged cleanup
    # failed: leaving files intact makes a retry possible.
    for name in ('wispnet-desktop.service', 'wispnet-host.service', 'wipsnet-host.service'):
        (units / name).unlink(missing_ok=True)
    for name in ('wispnet', 'wispnet-node', 'wispnet-desktop'):
        (home / '.local/bin' / name).unlink(missing_ok=True)
    (home / '.local/share/applications/wispnet.desktop').unlink(missing_ok=True)
    for name in ('wispnet-node', 'wispnet-desktop'):
        directory = home / '.local/libexec' / name
        if directory.is_symlink():
            directory.unlink()
        elif directory.exists():
            shutil.rmtree(directory)
    subprocess.run(['systemctl', '--user', 'daemon-reload'], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    if os.geteuid() == 0:
        parser.error('Run as your regular user, without sudo; firewall cleanup requests sudo separately')
    uninstall(Path.home(), os.getuid())
    print('Wispnet uninstalled. Configuration, identities, pairing and private state preserved.')


if __name__ == '__main__':
    main()
