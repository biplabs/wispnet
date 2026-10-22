#!/usr/bin/env python3
"""One-time administrator installation of Wispnet's socket-verified Linux firewall helper."""
import argparse
import json
import os
from pathlib import Path
import pwd
import shutil
import stat
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
CONFIG = Path('/etc/wispnet-firewall.json')
BINARY = Path('/usr/local/libexec/wispnet-firewall.py')
UNIT = Path('/etc/systemd/system/wispnet-firewall.service')


def write_root(path, content, mode):
    path.parent.mkdir(parents=True, exist_ok=True)
    # Never follow an administrator destination replaced by a symlink.
    for parent in [path.parent, *path.parent.parents]:
        m = parent.lstat()
        if not stat.S_ISDIR(m.st_mode) or m.st_uid != 0 or m.st_mode & 0o022:
            raise SystemExit('Unsafe destination directory: ' + str(parent))
    if path.is_symlink():
        raise SystemExit('Refusing symlink destination: ' + str(path))
    fd, temporary = tempfile.mkstemp(prefix='.wispnet-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as output:
            os.fchmod(output.fileno(), mode)
            output.write(content)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--uid', type=int, required=True, help='User running wispnet-host.service')
    parser.add_argument('--remove-legacy-port', type=int, help='Explicit old manual UFW UDP rule to remove after helper verification')
    args = parser.parse_args()
    if os.geteuid() != 0 or not 0 < args.uid < 2**32:
        parser.error('Run as administrator and specify a non-root user ID')
    pwd.getpwuid(args.uid)
    if args.remove_legacy_port is not None and not 1024 <= args.remove_legacy_port <= 65535:
        parser.error('Legacy UDP port must be unprivileged')
    from firewall_helper import read_json, Firewall
    Firewall()  # Fail before installation if no supported backend exists.
    uids = set()
    if CONFIG.exists() or CONFIG.is_symlink():
        value = read_json(CONFIG, 0)
        if value.get('version') != 1 or set(value) != {'version', 'uids'} or any(type(u) is not int or not 0 < u < 2**32 for u in value['uids']):
            parser.error('Invalid existing helper configuration')
        uids.update(value['uids'])
    uids.add(args.uid)
    if len(uids) > 32:
        parser.error('Too many configured users')
    write_root(BINARY, (ROOT / 'scripts/firewall_helper.py').read_text(), 0o755)
    write_root(CONFIG, json.dumps({'version': 1, 'uids': sorted(uids)})+'\n', 0o600)
    write_root(UNIT, (ROOT / 'deploy/node/wispnet-firewall.service').read_text(), 0o644)
    subprocess.run(['/usr/bin/systemctl', 'daemon-reload'], check=True)
    subprocess.run(['/usr/bin/systemctl', 'enable', 'wispnet-firewall.service'], check=True)
    subprocess.run(['/usr/bin/systemctl', 'restart', 'wispnet-firewall.service'], check=True)
    deadline = time.monotonic()+15
    while True:
        try:
            status = read_json(Path('/run/wispnet-firewall/status.json'), 0)
            ready = status['healthy'] and args.uid in status['uids'] and time.time()-status['updated'] < 5
            if args.remove_legacy_port is not None:
                ready = ready and any(p['uid'] == args.uid for p in status['ports'])
            if ready:
                break
        except (OSError, ValueError, KeyError):
            pass
        if time.monotonic() >= deadline:
            raise SystemExit('Helper did not become ready; inspect journalctl -u wispnet-firewall.service. Manual UFW rules were preserved.')
        time.sleep(.25)
    if args.remove_legacy_port is not None:
        ufw = next((str(Path(d)/'ufw') for d in ['/usr/sbin','/usr/bin','/sbin'] if (Path(d)/'ufw').exists()), None)
        if ufw is None:
            raise SystemExit('UFW not found; helper installed, legacy rule unchanged')
        subprocess.run([ufw, '--force', 'delete', 'allow', f'{args.remove_legacy_port}/udp'], check=True)
    print('Firewall helper installed. Dynamic node UDP ports are managed automatically.')


if __name__ == '__main__':
    main()
