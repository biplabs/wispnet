#!/usr/bin/env python3
"""Stage/install the optional Command Center without changing private Wispnet.

Requires an owner-only environment file with WISPNET_GITHUB_CLIENT_ID and
WISPNET_GITHUB_CLIENT_SECRET. This installer never prints their values and does
not change Caddy, gateway identity, publishers, or hosted service policy.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import stat
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
NODE_VERSION = '24.13.1'


def read_environment(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o600:
        raise ValueError('GitHub environment file must be owned, regular, mode 600 and not a link')
    values = {}
    for line in path.read_text().splitlines():
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        name, sep, value = line.partition('=')
        if not sep or name not in ('WISPNET_GITHUB_CLIENT_ID', 'WISPNET_GITHUB_CLIENT_SECRET'):
            raise ValueError('Environment file must contain only the two GitHub configuration values')
        parsed = shlex.split(value, comments=False)
        if len(parsed) != 1 or not parsed[0] or any(ord(c) < 32 for c in parsed[0]):
            raise ValueError('Invalid GitHub configuration value')
        values[name] = parsed[0]
    if set(values) != {'WISPNET_GITHUB_CLIENT_ID', 'WISPNET_GITHUB_CLIENT_SECRET'}:
        raise ValueError('Both GitHub configuration values are required')
    return values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ssh-host', required=True)
    parser.add_argument('--github-env', type=Path, required=True)
    parser.add_argument('--url', default='https://cmd.wispnet.app')
    parser.add_argument('--public-domain', default='wispnet.app')
    args = parser.parse_args()
    from urllib.parse import urlsplit
    url = urlsplit(args.url)
    if url.scheme != 'https' or not url.hostname or url.username or url.password or url.path not in ('', '/') or url.query or url.fragment:
        parser.error('--url must be an HTTPS origin')
    github = read_environment(args.github_env.absolute())

    def ssh(script):
        return subprocess.run(['ssh', '-o', 'BatchMode=yes', args.ssh_host, 'sh -c ' + shlex.quote(script)],
                              text=True, capture_output=True, check=True, timeout=300).stdout.strip()

    remote_home = ssh('printf "%s" "$HOME"')
    arch = ssh('uname -m')
    node_arch = {'aarch64': 'arm64', 'x86_64': 'x64'}.get(arch)
    if not node_arch:
        raise RuntimeError('Unsupported gateway architecture')
    app = remote_home + '/.local/libexec/wispnet-command-center'
    data = remote_home + '/.local/share/wispnet-command-center'
    config = remote_home + '/.config/wispnet-command-center'
    unit = remote_home + '/.config/systemd/user/wispnet-command-center.service'
    q = shlex.quote
    ssh('set -eu\numask 077\n' + '\n'.join('mkdir -p ' + q(p) for p in [app, data, data + '/state', config, str(Path(unit).parent)]) + '\n' + '\n'.join('chmod 700 ' + q(p) for p in [data, data + '/state', config]))

    filename = f'node-v{NODE_VERSION}-linux-{node_arch}.tar.xz'
    origin = f'https://nodejs.org/dist/v{NODE_VERSION}/'
    sums = urllib.request.urlopen(origin + 'SHASUMS256.txt', timeout=30).read().decode()
    expected = next(line.split()[0] for line in sums.splitlines() if line.split()[-1] == filename)
    archive = urllib.request.urlopen(origin + filename, timeout=90).read()
    if hashlib.sha256(archive).hexdigest() != expected:
        raise RuntimeError('Node archive checksum mismatch')
    with tempfile.TemporaryDirectory(prefix='wispnet-command-install-') as temporary:
        temp = Path(temporary)
        package = temp / 'command-center.tar.gz'
        with tarfile.open(package, 'w:gz') as tar:
            for name in ['package.json', 'package-lock.json', 'tsconfig.json', 'src', 'test']:
                path = ROOT / 'apps/auth-hub' / name
                if path.exists():
                    tar.add(path, arcname=name)
        node = temp / filename
        node.write_bytes(archive)
        def put(source, destination):
            subprocess.run(['scp', '-q', str(source), args.ssh_host + ':' + destination], check=True, timeout=90)
        put(package, app + '/source.tar.gz')
        put(node, data + '/node.tar.xz')
        put(ROOT / 'deploy/command-center/wispnet-command-center.service', unit)
        put(ROOT / 'deploy/command-center/Caddyfile.fragment', config + '/Caddyfile.fragment')
        ssh(f'set -eu\numask 077\nmkdir -p {q(data + "/node")}\ntar -xJf {q(data + "/node.tar.xz")} --strip-components=1 -C {q(data + "/node")}\ntar -xzf {q(app + "/source.tar.gz")} -C {q(app)}\ncd {q(app)}\nPATH={q(data + "/node/bin")}:"$PATH" npm ci --no-audit --no-fund\nPATH={q(data + "/node/bin")}:"$PATH" npm run build')
        # Keep the existing encryption/signing secret stable across upgrades.
        secret = ssh(f'if [ -f {q(config + "/secret")} ]; then cat {q(config + "/secret")}; else umask 077; python3 -c "import secrets; print(secrets.token_urlsafe(48))" > {q(config + "/secret")}; cat {q(config + "/secret")}; fi')
        environment = github | {'WISPNET_COMMAND_URL': args.url.rstrip('/'), 'WISPNET_COMMAND_STATE': data + '/state',
                                'WISPNET_COMMAND_SECRET': secret, 'WISPNET_PUBLIC_DOMAIN': args.public_domain, 'WISPNET_COMMAND_PORT': '8325'}
        envfile = temp / 'command.env'
        envfile.write_text(''.join(key + '=' + json.dumps(value) + '\n' for key, value in environment.items()))
        envfile.chmod(0o600)
        put(envfile, config + '/command.env.new')
        ssh(f'chmod 600 {q(config + "/command.env.new")}\nmv {q(config + "/command.env.new")} {q(config + "/command.env")}\nsystemctl --user daemon-reload\nsystemctl --user enable --now wispnet-command-center.service\nsystemctl --user restart wispnet-command-center.service')
        ssh("""python3 - <<'PY'
import json, time, urllib.request
deadline = time.monotonic() + 20
while True:
    try:
        with urllib.request.urlopen('http://127.0.0.1:8325/healthz', timeout=2) as response:
            if response.status == 200 and json.load(response).get('ok') is True:
                break
    except (OSError, ValueError):
        pass
    if time.monotonic() >= deadline:
        raise SystemExit('Command Center did not become healthy; inspect the protected service configuration')
    time.sleep(0.5)
PY""")
    print('Command Center installed; add the generated Caddyfile.fragment to the existing HTTPS configuration.')
    print('Then upgrade the public gateway and enable its command_center configuration before publishing GIKE.')


if __name__ == '__main__':
    main()
