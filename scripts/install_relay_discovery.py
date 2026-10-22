#!/usr/bin/env python3
"""Enable authenticated UDP discovery beside the Caddy-fronted packet relay.

Preserves the HTTPS front end, node identities, and public-service configuration.
Caddy certificate changes restart the relay through a systemd path watcher.
"""
import argparse
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]


def discovery_config(text, certificate, key):
    current = tomllib.loads(text)
    if not current.get('http_only') or not current.get('listen', '').startswith('127.0.0.1:'):
        raise ValueError('This installer requires the existing loopback HTTP relay behind Caddy')
    fields = {'quic_listen', 'quic_certificate', 'quic_private_key'}
    lines = [line for line in text.splitlines() if line.split('=', 1)[0].strip() not in fields]
    for name, value in [('quic_listen', '0.0.0.0:7842'),
                        ('quic_certificate', certificate), ('quic_private_key', key)]:
        lines.append(name + '=' + json.dumps(value))
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ssh-host', required=True)
    parser.add_argument('--skip-build', action='store_true')
    args = parser.parse_args()
    if args.ssh_host.startswith('-') or not all(c.isalnum() or c in '.-_@' for c in args.ssh_host):
        parser.error('Invalid SSH host')
    q = shlex.quote

    def remote(script):
        return subprocess.run(['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10',
                               args.ssh_host, 'sh -c ' + q(script)], check=True,
                              capture_output=True, text=True, timeout=40).stdout.strip()

    if not args.skip_build:
        subprocess.run(['cargo', 'build', '-p', 'wispnet-relay', '--locked'], cwd=ROOT, check=True)
    home = remote('printf "%s" "$HOME"')
    directory = home + '/.local/share/wispnet-public-https/caddy/certificates'
    certificate = remote('find ' + q(directory) + ' -type f -name relay.wispnet.app.crt')
    if not certificate.startswith(directory + '/') or '\n' in certificate:
        raise RuntimeError('Expected exactly one Caddy relay certificate')
    key = certificate[:-4] + '.key'
    remote('test -r ' + q(certificate) + ' && test -r ' + q(key))
    config_path = home + '/.config/wispnet-relay/relay.toml'
    config = discovery_config(remote('cat ' + q(config_path)), certificate, key)
    binary = home + '/.local/libexec/wispnet-relay/wispnet-relay'
    units = home + '/.config/systemd/user/'
    watcher = units + 'wispnet-relay-certificate.path'
    reload = units + 'wispnet-relay-certificate.service'
    paths = [binary, config_path, watcher, reload]
    backup = home + '/.local/state/wispnet-relay-discovery/' + time.strftime('%Y%m%d-%H%M%S-') + uuid.uuid4().hex[:8]
    remote('umask 077\nmkdir -p ' + q(backup))
    existed = []
    for i, path in enumerate(paths):
        exists = remote('if [ -f ' + q(path) + ' ]; then cp -p ' + q(path) + ' ' + q(backup + '/' + str(i)) + '; printf yes; fi') == 'yes'
        existed.append(exists)
    watcher_enabled = remote('systemctl --user is-enabled wispnet-relay-certificate.path 2>/dev/null || true') == 'enabled'

    def put(source, destination, mode='600'):
        staging = destination + '.udp-next'
        subprocess.run(['scp', '-q', str(source), args.ssh_host + ':' + staging], check=True, timeout=40)
        remote('chmod ' + mode + ' ' + q(staging) + ' && mv ' + q(staging) + ' ' + q(destination))

    try:
        with tempfile.TemporaryDirectory(prefix='wispnet-relay-discovery-') as temporary:
            temporary = Path(temporary)
            candidate = temporary / 'wispnet-relay'
            shutil.copyfile(ROOT / 'target/debug/wispnet-relay', candidate)
            subprocess.run(['strip', str(candidate)], check=True)
            put(candidate, binary, '755')
            source = temporary / 'relay.toml'
            source.write_text(config)
            put(source, config_path)
            source = temporary / 'certificate.path'
            source.write_text('[Unit]\nDescription=Watch Wispnet relay discovery certificate renewal\n[Path]\n'
                              f'PathChanged={certificate}\nPathChanged={key}\n'
                              'Unit=wispnet-relay-certificate.service\n[Install]\nWantedBy=default.target\n')
            put(source, watcher)
            source = temporary / 'certificate.service'
            source.write_text('[Unit]\nDescription=Reload Wispnet relay after certificate renewal\n[Service]\n'
                              'Type=oneshot\nExecStart=/usr/bin/systemctl --user restart wispnet-relay.service\n')
            put(source, reload)
            remote('systemctl --user daemon-reload && systemctl --user enable --now wispnet-relay-certificate.path && systemctl --user restart wispnet-relay.service')
            time.sleep(2)
            remote('systemctl --user is-active --quiet wispnet-relay.service && systemctl --user is-active --quiet wispnet-relay-certificate.path')
            listeners = remote('ss -lun')
            if '0.0.0.0:7842' not in listeners:
                raise RuntimeError('UDP discovery listener did not start')
            remote('curl --fail --silent --max-time 8 https://relay.wispnet.app/ping >/dev/null')
    except Exception:
        remote('systemctl --user disable --now wispnet-relay-certificate.path || true')
        for i, path in enumerate(paths):
            if existed[i]:
                remote('cp -p ' + q(backup + '/' + str(i)) + ' ' + q(path + '.udp-rollback') + ' && mv ' + q(path + '.udp-rollback') + ' ' + q(path))
            elif i >= 2:
                remote('rm -f ' + q(path))
        remote('systemctl --user daemon-reload')
        if watcher_enabled:
            remote('systemctl --user enable --now wispnet-relay-certificate.path')
        remote('systemctl --user restart wispnet-relay.service')
        raise
    print(json.dumps({'event': 'relay_discovery_installed', 'udp_port': 7842,
                      'backup': backup, 'certificate_renewal_watcher': True}))


if __name__ == '__main__':
    main()
