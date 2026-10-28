#!/usr/bin/env python3
"""Install the Rust Command Center without changing private Wispnet policy.

Use --use-remote-env to keep the complete protected configuration and GitHub
credentials on the gateway. Alternatively, --github-env explicitly stages a
local mode-600 file containing the two GitHub values. A cross-architecture
gateway needs --binary pointing to its separately built Linux ELF executable.
"""
import argparse
from contextlib import nullcontext
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
    if len(content) > 16384:
        raise ValueError('GitHub environment file is too large')
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


REMOTE_ENV_CHECK = """# wispnet-command-remote-environment
import os, shlex, stat, sys
path, expected_url, expected_domain, expected_state, destination = sys.argv[1:]
try:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, 'rb') as source:
        info = os.fstat(source.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
                or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o600
                or info.st_size > 16384):
            raise ValueError('Remote command.env must be an owned mode-600 regular single-link file')
        content = source.read(16385)
    if len(content) > 16384:
        raise ValueError('Remote command.env is too large')
    allowed = {'WISPNET_COMMAND_URL', 'WISPNET_COMMAND_STATE', 'WISPNET_GITHUB_CLIENT_ID',
               'WISPNET_GITHUB_CLIENT_SECRET', 'WISPNET_PUBLIC_DOMAIN', 'WISPNET_COMMAND_PORT'}
    values = {}
    for line in content.decode('utf-8').splitlines():
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        key, separator, value = line.partition('=')
        if not separator or key not in allowed or key in values:
            raise ValueError('Remote command.env must contain each of the six configuration values exactly once')
        parsed = shlex.split(value, comments=False)
        if len(parsed) != 1 or not parsed[0] or any(ord(c) < 32 or ord(c) == 127 for c in parsed[0]):
            raise ValueError('Complete all six remote configuration values, including both GitHub values')
        values[key] = parsed[0]
    if set(values) != allowed:
        raise ValueError('Complete all six remote configuration values, including both GitHub values')
    expected = {'WISPNET_COMMAND_URL': expected_url, 'WISPNET_COMMAND_STATE': expected_state,
                'WISPNET_PUBLIC_DOMAIN': expected_domain, 'WISPNET_COMMAND_PORT': '8325'}
    if any(values[key] != value for key, value in expected.items()):
        raise ValueError('Remote command.env URL, state, domain and port must match this installation')
    if destination:
        descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, 'wb') as staged:
            staged.write(content)
            staged.flush()
            os.fsync(staged.fileno())
except (OSError, UnicodeError, ValueError):
    raise SystemExit('Remote command.env is unsafe, incomplete, or does not match the installation; edit it on the gateway') from None
"""


def remote_home_directory(remote):
    remote_home = remote.run('printf "%s" "$HOME"')
    if not remote_home.startswith('/') or any(ord(c) < 32 or ord(c) == 127 for c in remote_home):
        raise ValueError('Invalid remote home directory')
    return remote_home


def remote_environment(remote, remote_home, url, public_domain, destination=''):
    """Validate/copy on the gateway; stdout and SSH arguments contain no secrets."""
    config = remote_home + '/.config/wispnet-command-center/command.env'
    state = remote_home + '/.local/share/wispnet-command-center/state'
    arguments = [config, url.rstrip('/'), public_domain, state, destination]
    script = 'python3 - ' + ' '.join(shlex.quote(value) for value in arguments)
    script += " <<'WISPNET_REMOTE_ENV'\n" + REMOTE_ENV_CHECK + '\nWISPNET_REMOTE_ENV'
    try:
        remote.run(script)
    except subprocess.CalledProcessError:
        raise ValueError('Remote command.env must be protected, complete and match the requested installation; edit it on the gateway') from None


def deploy(remote, binary, github, url, public_domain):
    remote_home = remote_home_directory(remote)
    if binary_architecture(binary) != ARCHITECTURES.get(remote.run('uname -m')):
        raise ValueError('Command Center binary architecture does not match the gateway')
    app = remote_home + '/.local/libexec/wispnet-command-center'
    data = remote_home + '/.local/share/wispnet-command-center'
    config = remote_home + '/.config/wispnet-command-center'
    unit = remote_home + '/.config/systemd/user/' + SERVICE
    executable = app + '/wispnet-command-center'
    environment = config + '/command.env'
    if github is None:
        # Fail before creating directories, uploading files or changing service
        # state. A second validation below copies from the same protected fd.
        remote_environment(remote, remote_home, url, public_domain)
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
        local_staging = tempfile.TemporaryDirectory(prefix='wispnet-command-install-') if github is not None else nullcontext()
        with local_staging as temporary:
            uploads = [(binary, 'binary'),
                       (ROOT / 'deploy/command-center' / SERVICE, 'unit'),
                       (ROOT / 'deploy/command-center/Caddyfile.fragment', 'Caddyfile.fragment')]
            if github is None:
                remote_environment(remote, remote_home, url, public_domain, staging + '/environment')
            else:
                envfile = Path(temporary) / 'command.env'
                values = github | {'WISPNET_COMMAND_URL': url.rstrip('/'),
                                   'WISPNET_COMMAND_STATE': data + '/state',
                                   'WISPNET_PUBLIC_DOMAIN': public_domain,
                                   'WISPNET_COMMAND_PORT': '8325'}
                envfile.write_text(''.join(key + '=' + json.dumps(value) + '\n' for key, value in values.items()))
                envfile.chmod(0o600)
                uploads.append((envfile, 'environment'))
            for source, name in uploads:
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
    credentials = parser.add_mutually_exclusive_group(required=True)
    credentials.add_argument('--use-remote-env', action='store_true', help='Use existing protected command.env on the gateway; keep credentials remote')
    credentials.add_argument('--github-env', type=Path, help='Explicitly stage a local file containing the two GitHub configuration values')
    parser.add_argument('--binary', type=Path, help='Prebuilt Linux executable matching the gateway architecture')
    parser.add_argument('--url', default='https://cmd.wispnet.app')
    parser.add_argument('--public-domain', default='wispnet.app')
    args = parser.parse_args()
    url = urlsplit(args.url)
    if url.scheme != 'https' or not url.hostname or url.username or url.password or url.path not in ('', '/') or url.query or url.fragment:
        parser.error('--url must be an HTTPS origin')
    github = read_environment(args.github_env.absolute()) if args.github_env else None
    remote = Remote(args.ssh_host)
    if args.use_remote_env:
        remote_environment(remote, remote_home_directory(remote), args.url, args.public_domain)
    binary = select_binary(remote.run('uname -m'), args.binary)
    deploy(remote, binary, github, args.url, args.public_domain)
    print('Rust Command Center installed; add the generated Caddyfile.fragment to the existing HTTPS configuration.')
    print('Then upgrade the public gateway and enable its command_center configuration before publishing GIKE.')


if __name__ == '__main__':
    main()
