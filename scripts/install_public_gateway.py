#!/usr/bin/env python3
"""Install the public gateway and HTTPS front end beside the existing packet relay.

DNS is intentionally operator-managed. This never publishes a backend service.
The existing relay is backed up and restored if HTTPS startup/health fails.
"""
import argparse
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import urllib.request
import urllib.error

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ssh-host', required=True)
    parser.add_argument('--publisher', action='append', required=True, help='Allowed owning node endpoint ID; repeat for each owner')
    parser.add_argument('--skip-build', action='store_true')
    args = parser.parse_args()
    if any(len(p) != 64 or any(c not in '0123456789abcdef' for c in p) for p in args.publisher):
        parser.error('Publishers must be full lowercase endpoint IDs')

    def ssh(script, **kwargs):
        return subprocess.run(['ssh', '-o', 'BatchMode=yes', args.ssh_host, 'sh -c ' + shlex.quote(script)], check=True,
                              capture_output=True, text=True, timeout=90, **kwargs).stdout.strip()

    home = ssh('printf "%s" "$HOME"')
    quote = shlex.quote
    config = home + '/.config/wispnet-public'
    state = home + '/.local/share/wispnet-public-gateway'
    cache = home + '/.local/share/wispnet-public-https'
    backup = home + '/.local/state/wispnet-public-upgrade/' + time.strftime('%Y%m%d-%H%M%S')
    if not args.skip_build:
        subprocess.run(['cargo', 'build', '-p', 'wispnet-public-gateway', '-p', 'wispnet-relay', '--locked'], cwd=ROOT, check=True)
    files = {
        home + '/.local/libexec/wispnet-relay/wispnet-relay': ROOT / 'target/debug/wispnet-relay',
        home + '/.local/libexec/wispnet-public-gateway/wispnet-public-gateway': ROOT / 'target/debug/wispnet-public-gateway',
        config + '/Caddyfile': ROOT / 'deploy/public/Caddyfile',
        home + '/.config/systemd/user/wispnet-public-gateway.service': ROOT / 'deploy/public/wispnet-public-gateway.service',
        home + '/.config/systemd/user/wispnet-public-https.service': ROOT / 'deploy/public/wispnet-public-https.service',
    }
    old_front = ssh('systemctl --user is-active wispnet-public-https.service || true') == 'active'
    relay_config = home + '/.config/wispnet-relay/relay.toml'
    paths = list(files) + [config + '/gateway.toml', relay_config]
    ssh('set -eu\numask 077\n' + '\n'.join(f'mkdir -p {quote(p)}' for p in [config, state, cache, cache + '/config', backup, home + '/.local/libexec/wispnet-public-gateway']))
    for index, path in enumerate(paths):
        ssh(f'if [ -f {quote(path)} ]; then cp -p {quote(path)} {quote(backup + "/" + str(index))}; fi')

    def put(local, remote, executable=False):
        staging = remote + '.new'
        subprocess.run(['scp', '-q', str(local), args.ssh_host + ':' + staging], check=True, timeout=90)
        ssh(f'chmod {"755" if executable else "600"} {quote(staging)} && mv {quote(staging)} {quote(remote)}')

    try:
        with tempfile.TemporaryDirectory(prefix='wispnet-public-deploy-') as temporary:
            temporary = Path(temporary)
            for remote, source in files.items():
                executable = '/libexec/' in remote
                if executable:
                    binary = temporary / source.name
                    shutil.copyfile(source, binary)
                    if shutil.which('strip'):
                        subprocess.run(['strip', str(binary)], check=True)
                    put(binary, remote, True)
                else:
                    put(source, remote)
            gateway_id = ssh(f'{quote(home + "/.local/libexec/wispnet-public-gateway/wispnet-public-gateway")} --init {quote(state)}')
            bootstrap = json.loads((ROOT / 'deploy/public/gateway.json').read_text())
            if not bootstrap or gateway_id != bootstrap['endpoint_id']:
                raise RuntimeError('Gateway identity does not match deploy/public/gateway.json; preserve the identity and update the public bootstrap before deployment')
            gateway = temporary / 'gateway.toml'
            gateway.write_text(f'state_dir={json.dumps(state)}\nlisten="127.0.0.1:18080"\ndomain="wispnet.app"\npublishers={json.dumps(sorted(set(args.publisher)))}\n[transport]\nbind_addr="127.0.0.1:0"\nrelay_url="https://relay.wispnet.app"\nrelay_only=true\n')
            put(gateway, config + '/gateway.toml')
            ssh(f'caddy adapt --config {quote(config + "/Caddyfile")} --adapter caddyfile >/dev/null')
            relay = temporary / 'relay.toml'
            relay.write_text('listen="127.0.0.1:18443"\nhttp_only=true\nmax_connections=64\n')
            put(relay, relay_config)
            relay_changed = any(ssh(f'cmp -s {quote(backup + "/" + str(paths.index(path)))} {quote(path)} || printf changed') == 'changed' for path in [home + '/.local/libexec/wispnet-relay/wispnet-relay', relay_config])
            front_changed = not old_front or any(ssh(f'cmp -s {quote(backup + "/" + str(paths.index(path)))} {quote(path)} || printf changed') == 'changed' for path in [config + '/Caddyfile', home + '/.config/systemd/user/wispnet-public-https.service'])
            ssh('systemctl --user daemon-reload && systemctl --user enable wispnet-public-gateway.service wispnet-public-https.service && systemctl --user restart wispnet-public-gateway.service')
            if relay_changed:
                ssh('systemctl --user restart wispnet-relay.service')
            if front_changed:
                ssh('systemctl --user restart wispnet-public-https.service')
            deadline = time.monotonic() + 65
            while True:
                try:
                    response = urllib.request.urlopen('https://relay.wispnet.app/', timeout=6)
                    status = response.status
                    response.close()
                except urllib.error.HTTPError as error:
                    status = error.code
                    error.close()
                except (OSError, urllib.error.URLError):
                    status = 0
                if status and status not in (502, 503):
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError('Public HTTPS relay health did not recover')
                time.sleep(1)
            for unit in ['wispnet-relay.service', 'wispnet-public-gateway.service', 'wispnet-public-https.service']:
                if ssh(f'systemctl --user is-active {quote(unit)}') != 'active':
                    raise RuntimeError(unit + ' is not active')
            print('Public gateway installed; existing relay HTTPS verified; no services published.')
            print('Publisher IDs: ' + ', '.join(sorted(set(args.publisher))))
            print('Rollback snapshot: ' + backup)
    except Exception:
        ssh('systemctl --user stop wispnet-public-https.service wispnet-public-gateway.service || true')
        for index, path in enumerate(paths):
            ssh(f'if [ -f {quote(backup + "/" + str(index))} ]; then cp -p {quote(backup + "/" + str(index))} {quote(path)}; else rm -f {quote(path)}; fi')
        ssh('systemctl --user daemon-reload && systemctl --user restart wispnet-relay.service')
        if old_front:
            ssh('systemctl --user restart wispnet-public-gateway.service wispnet-public-https.service')
        raise


if __name__ == '__main__':
    main()
