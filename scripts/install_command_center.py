#!/usr/bin/env python3
"""Install the Rust Command Center without changing private Wispnet policy.

Provide a mode-600 file containing only WISPNET_GITHUB_CLIENT_ID and
WISPNET_GITHUB_CLIENT_SECRET. A cross-architecture gateway needs --binary
pointing to a separately built Linux ELF executable for that machine.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shlex
import stat
import struct
import subprocess
import tempfile
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
SERVICE = 'wispnet-command-center.service'
ARCHITECTURES = {'x86_64': 'x86_64', 'amd64': 'x86_64', 'aarch64': 'aarch64', 'arm64': 'aarch64'}


def read_environment(path):
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except OSError:
        raise ValueError('GitHub environment file must be owned, regular, mode 600 and not a link') from None
    with os.fdopen(descriptor) as source:
        info = os.fstat(source.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
                or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o600
                or info.st_size > 16384):
            raise ValueError('GitHub environment file must be owned, regular, mode 600 and not a link')
        content = source.read(16385)
    values = {}
    for line in content.splitlines():
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        name, sep, value = line.partition('=')
        if (not sep or name in values
                or name not in ('WISPNET_GITHUB_CLIENT_ID', 'WISPNET_GITHUB_CLIENT_SECRET')):
            raise ValueError('Environment file must contain each GitHub configuration value exactly once')
        parsed = shlex.split(value, comments=False)
        if len(parsed) != 1 or not parsed[0] or any(ord(c) < 32 or ord(c) == 127 for c in parsed[0]):
            raise ValueError('Invalid GitHub configuration value')
        values[name] = parsed[0]
    if set(values) != {'WISPNET_GITHUB_CLIENT_ID', 'WISPNET_GITHUB_CLIENT_SECRET'}:
        raise ValueError('Both GitHub configuration values are required')
    return values


def binary_architecture(path):
    """Check the executable itself, rather than trusting its filename."""
    with path.open('rb') as source:
        header = source.read(20)
    if (len(header) != 20 or header[:4] != b'\x7fELF'
            or header[4] != 2 or header[5] not in (1, 2) or header[6] != 1):
        raise ValueError('Command Center binary must be a 64-bit Linux ELF executable')
    kind, machine = struct.unpack(('<' if header[5] == 1 else '>') + 'HH', header[16:20])
    if kind not in (2, 3) or machine not in (62, 183):
        raise ValueError('Command Center binary must target Linux x86_64 or aarch64')
    return {62: 'x86_64', 183: 'aarch64'}[machine]


def select_binary(remote_arch, supplied=None):
    wanted = ARCHITECTURES.get(remote_arch)
    if not wanted:
        raise ValueError('Unsupported gateway architecture')
    if supplied is None:
        if platform.system() != 'Linux' or ARCHITECTURES.get(platform.machine()) != wanted:
            raise ValueError('Cross-architecture deployment requires --binary with a separately built Linux executable for the gateway')
        subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'wispnet-command-center'], cwd=ROOT, check=True)
        supplied = ROOT / 'target/release/wispnet-command-center'
    supplied = supplied.absolute()
    if binary_architecture(supplied) != wanted:
        raise ValueError('Command Center binary architecture does not match the gateway')
    return supplied


class Remote:
    def __init__(self, host):
        if not host or host.startswith('-') or any(c.isspace() or ord(c) < 32 for c in host):
            raise ValueError('Invalid SSH host')
        self.host = host

    def run(self, script):
        return subprocess.run(['ssh', '-o', 'BatchMode=yes', self.host,
                               'sh -c ' + shlex.quote(script)],
                              text=True, capture_output=True, check=True, timeout=90).stdout.strip()

    def put(self, source, destination):
        subprocess.run(['scp', '-q', str(source), self.host + ':' + destination],
                       check=True, timeout=90)


HEALTH_CHECK = """# wispnet-command-health-check
python3 - <<'PY'
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
        raise SystemExit('Command Center did not become healthy')
    time.sleep(0.5)
PY
"""


def deploy(remote, binary, github, url, public_domain):
    remote_home = remote.run('printf "%s" "$HOME"')
    if not remote_home.startswith('/') or any(ord(c) < 32 or ord(c) == 127 for c in remote_home):
        raise ValueError('Invalid remote home directory')
    if binary_architecture(binary) != ARCHITECTURES.get(remote.run('uname -m')):
        raise ValueError('Command Center binary architecture does not match the gateway')
    app = remote_home + '/.local/libexec/wispnet-command-center'
    data = remote_home + '/.local/share/wispnet-command-center'
    config = remote_home + '/.config/wispnet-command-center'
    unit = remote_home + '/.config/systemd/user/' + SERVICE
    executable = app + '/wispnet-command-center'
    environment = config + '/command.env'
    q = shlex.quote
    directories = [app, data, data + '/state', config, str(Path(unit).parent)]
    remote.run('set -eu\numask 077\n' + '\n'.join('mkdir -p ' + q(p) for p in directories)
               + '\n' + '\n'.join('chmod 700 ' + q(p) for p in directories[:-1]))
    staging = remote.run('umask 077\nmktemp -d ' + q(app + '/.install.XXXXXXXX'))
    if (not staging.startswith(app + '/.install.') or '/' in staging[len(app) + 1:]
            or any(ord(c) < 32 or ord(c) == 127 for c in staging)):
        raise ValueError('Invalid remote staging directory')
    replaced = False
    cleanup = True
    try:
        with tempfile.TemporaryDirectory(prefix='wispnet-command-install-') as temporary:
            envfile = Path(temporary) / 'command.env'
            values = github | {'WISPNET_COMMAND_URL': url.rstrip('/'),
                               'WISPNET_COMMAND_STATE': data + '/state',
                               'WISPNET_PUBLIC_DOMAIN': public_domain,
                               'WISPNET_COMMAND_PORT': '8325'}
            envfile.write_text(''.join(key + '=' + json.dumps(value) + '\n' for key, value in values.items()))
            envfile.chmod(0o600)
            for source, name in [(binary, 'binary'), (envfile, 'environment'),
                                 (ROOT / 'deploy/command-center' / SERVICE, 'unit'),
                                 (ROOT / 'deploy/command-center/Caddyfile.fragment', 'Caddyfile.fragment')]:
                remote.put(source, staging + '/' + name)
            with binary.open('rb') as source:
                expected = hashlib.file_digest(source, 'sha256').hexdigest()
            received = remote.run('sha256sum ' + q(staging + '/binary')).split()[0]
            if received != expected:
                raise RuntimeError('Uploaded Command Center binary checksum mismatch')
            # Backups remain inside mode-700 staging. Never replace the database.
            backup = ['set -eu', 'umask 077']
            for name, path in [('binary', executable), ('environment', environment), ('unit', unit)]:
                backup.append(f'if [ -e {q(path)} ]; then cp -p {q(path)} {q(staging + "/previous." + name)}; fi')
            backup += [f'if systemctl --user is-active --quiet {SERVICE}; then touch {q(staging + "/was-active")}; fi',
                       f'if systemctl --user is-enabled --quiet {SERVICE}; then touch {q(staging + "/was-enabled")}; fi']
            remote.run('\n'.join(backup))
            replaced = True
            remote.run('\n'.join(['set -eu', 'umask 077',
                'chmod 700 ' + q(staging + '/binary'),
                'chmod 600 ' + q(staging + '/environment') + ' ' + q(staging + '/unit'),
                f'mv {q(staging + "/binary")} {q(executable)}',
                f'mv {q(staging + "/environment")} {q(environment)}',
                f'mv {q(staging + "/unit")} {q(unit)}',
                'systemctl --user daemon-reload',
                f'systemctl --user enable {SERVICE}',
                f'systemctl --user restart {SERVICE}']))
            remote.run(HEALTH_CHECK)
            remote.run('chmod 600 ' + q(staging + '/Caddyfile.fragment') + '\nmv '
                       + q(staging + '/Caddyfile.fragment') + ' ' + q(config + '/Caddyfile.fragment'))
    except Exception:
        if replaced:
            rollback = ['set -eu', 'umask 077', f'systemctl --user stop {SERVICE} || true']
            for name, path in [('binary', executable), ('environment', environment), ('unit', unit)]:
                previous = staging + '/previous.' + name
                rollback.append(f'if [ -e {q(previous)} ]; then mv {q(previous)} {q(path)}; else rm -f {q(path)}; fi')
            rollback += ['systemctl --user daemon-reload',
                         f'if [ -e {q(staging + "/was-enabled")} ]; then systemctl --user enable {SERVICE}; else systemctl --user disable {SERVICE}; fi',
                         f'if [ -e {q(staging + "/was-active")} ]; then systemctl --user start {SERVICE}; fi']
            try:
                remote.run('\n'.join(rollback))
            except Exception:
                cleanup = False
                raise RuntimeError('Command Center installation and rollback failed; inspect the service configuration') from None
        raise RuntimeError('Command Center installation failed; previous executable and configuration restored') from None
    finally:
        # Restrict deletion to the staging directory validated above.
        if cleanup:
            remote.run('rm -rf -- ' + q(staging))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ssh-host', required=True)
    parser.add_argument('--github-env', type=Path, required=True)
    parser.add_argument('--binary', type=Path, help='Prebuilt Linux executable matching the gateway architecture')
    parser.add_argument('--url', default='https://cmd.wispnet.app')
    parser.add_argument('--public-domain', default='wispnet.app')
    args = parser.parse_args()
    url = urlsplit(args.url)
    if url.scheme != 'https' or not url.hostname or url.username or url.password or url.path not in ('', '/') or url.query or url.fragment:
        parser.error('--url must be an HTTPS origin')
    github = read_environment(args.github_env.absolute())
    remote = Remote(args.ssh_host)
    binary = select_binary(remote.run('uname -m'), args.binary)
    deploy(remote, binary, github, args.url, args.public_domain)
    print('Rust Command Center installed; add the generated Caddyfile.fragment to the existing HTTPS configuration.')
    print('Then upgrade the public gateway and enable its command_center configuration before publishing GIKE.')


if __name__ == '__main__':
    main()
