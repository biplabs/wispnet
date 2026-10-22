#!/usr/bin/env python3
"""Fixed setup/removal operations for the pacman-owned Wispnet installation."""
import argparse
import json
import os
from pathlib import Path
import pwd
import socket
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from firewall_helper import CONFIG, Firewall, configured_uids, read_json
from install_firewall_helper import write_root

UNIT = 'wispnet-firewall.service'


def run(*args):
    return subprocess.run(list(args), check=True)


def configure_firewall(uid):
    if os.geteuid() != 0 or not 0 < uid < 2**32:
        raise ValueError('Use sudo and specify a non-root UID')
    pwd.getpwuid(uid)
    override = Path('/etc/systemd/system/' + UNIT)
    if override.exists() or override.is_symlink():
        raise ValueError('A standalone firewall helper overrides the package. Remove that helper with its uninstaller first.')
    uids = configured_uids() if CONFIG.exists() else set()
    uids.add(uid)
    if len(uids) > 32:
        raise ValueError('Too many configured users')
    Firewall()  # Validate backend before changing configuration.
    write_root(CONFIG, json.dumps({'version': 1, 'uids': sorted(uids)})+'\n', 0o600)
    run('/usr/bin/systemctl', 'enable', '--now', UNIT)
    run('/usr/bin/systemctl', 'restart', UNIT)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            status = read_json(Path('/run/wispnet-firewall/status.json'), 0)
            if status['healthy'] and uid in status['uids'] and time.time()-status['updated'] < 5:
                return
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(.25)
    raise RuntimeError('Firewall helper did not become ready; inspect journalctl -u '+UNIT)


def remove():
    if os.geteuid() != 0:
        raise ValueError('Package cleanup requires root')
    # Stop only services actually loaded from this package's system user units.
    # No identity/configuration paths under any user's home are touched.
    runtime_root = Path('/run/user')
    for runtime in runtime_root.iterdir() if runtime_root.exists() else ():
        if not runtime.name.isdecimal() or not (runtime/'systemd/private').exists():
            continue
        user = pwd.getpwuid(int(runtime.name)).pw_name
        prefix = ['/usr/bin/systemctl', '--user', '--machine', user+'@.host']
        for unit in ('wispnet-desktop.service', 'wispnet-host.service'):
            result = subprocess.run([*prefix, 'show', '--property=FragmentPath', '--value', unit],
                                    check=True, text=True, capture_output=True)
            if result.stdout.strip() == '/usr/lib/systemd/user/'+unit:
                run(*prefix, 'stop', unit)
    run('/usr/bin/systemctl', 'disable', '--now', UNIT)
    if subprocess.run(['/usr/bin/systemctl', 'is-active', '--quiet', UNIT]).returncode == 0:
        raise RuntimeError('Firewall helper is still running')
    Firewall().reconcile(set(), cleanup=True)
    CONFIG.unlink(missing_ok=True)


def setup(headless=False):
    if os.geteuid() == 0:
        raise ValueError('Run wispnet-setup as your regular user')
    for name in ('wispnet-host.service', 'wispnet-desktop.service'):
        path = Path.home()/'.config/systemd/user'/name
        if path.exists() or path.is_symlink():
            raise ValueError('A standalone user service overrides the package. Run its uninstaller first; config and identity are preserved.')
    config = Path.home()/'.config/wispnet-node/host.toml'
    if not config.exists():
        legacy = Path.home()/'.config/wipsnet-node/host.toml'
        config.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if legacy.exists():
            # Preserve the original state location and authority on migration.
            contents = legacy.read_text()
        else:
            state = Path.home()/'.local/state/wispnet-node'
            contents = ('schema_version=2\ndevice_name='+json.dumps(socket.gethostname())+
                        '\nstate_dir='+json.dumps(str(state))+'\nlog_level="info"\n'
                        '[transport]\nbind_addr="0.0.0.0:0"\n'
                        'relay_url="https://relay.wispnet.app"\n'
                        'backup_relay_urls=["https://usw1-1.relay.n0.iroh.link"]\n')
        with config.open('x') as output:
            os.fchmod(output.fileno(), 0o600)
            output.write(contents)
        if not legacy.exists():
            run('/usr/bin/wispnet', 'host-init', '--config', str(config))
    # Refuse missing/broken authority; never reinitialize an existing config.
    run('/usr/bin/wispnet', 'host-status', '--config', str(config))
    run('/usr/bin/systemctl', '--user', 'daemon-reload')
    run('/usr/bin/systemctl', '--user', 'enable', '--now', 'wispnet-host.service')
    if not headless:
        run('/usr/bin/systemctl', '--user', 'enable', '--now', 'wispnet-desktop.service')
    print('Wispnet configured. Optional firewall setup: sudo wispnet-setup --firewall-uid '+str(os.getuid()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument('--firewall-uid', type=int)
    modes.add_argument('--remove', action='store_true')
    modes.add_argument('--headless', action='store_true')
    args = parser.parse_args()
    if args.firewall_uid is not None:
        configure_firewall(args.firewall_uid)
    elif args.remove:
        remove()
    else:
        setup(args.headless)


if __name__ == '__main__':
    main()
