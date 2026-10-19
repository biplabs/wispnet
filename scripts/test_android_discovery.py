#!/usr/bin/env python3
"""Exercise discovery from a separate-UID sample app on an attached Android device.

Uses a temporary service and separately sandboxed peercheck app; leaves the
user's installed app, existing device grants and host/relay services untouched.
Build APKs first; see docs/android-service-discovery.md.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
PKG = 'app.wispnet.android.peercheck'
EXAMPLE = 'app.wispnet.discoveryexample'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True)
    args = parser.parse_args()
    def run(command, **kw):
        return subprocess.run(command, check=True, text=True, capture_output=True, timeout=60, **kw)
    def adb(*command, **kw):
        return run(['adb', '-s', args.serial, *command], **kw)
    binary = Path.home() / '.local/libexec/wispnet-node/wispnet'
    config = Path.home() / '.config/wispnet-node/host.toml'
    def cli(op, *tail):
        text = run([str(binary), op, '--config', str(config), *map(str, tail)]).stdout.strip()
        return json.loads(text) if text and op != 'host-mutate' else None
    host = cli('host-status')['host']
    service = 'discovery-' + uuid.uuid4().hex[:12]
    class Backend(BaseHTTPRequestHandler):
        def do_GET(self):
            assert self.headers.get('X-Wispnet-Access') is None, 'Credential leaked upstream'
            self.send_response(200)
            self.send_header('Content-Length', '2')
            self.end_headers()
            self.wfile.write(b'ok')
        def log_message(self, *args):
            pass
    server = ThreadingHTTPServer(('127.0.0.1', 0), Backend)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    peer = None
    process = None
    registered = []
    issued = None
    with tempfile.TemporaryDirectory(prefix='wispnet-discovery-') as temporary:
        root = Path(temporary)
        def mutate(value):
            value['expected_revision'] = cli('host-status')['revision']
            path = root / 'request.json'
            path.write_text(json.dumps(value));path.chmod(0o600)
            return cli('host-mutate', '--request', path)
        try:
            adb('install', '-r', str(ROOT / 'apps/android/app/build/outputs/apk/peercheck/app-peercheck.apk'))
            adb('install', '-r', str(ROOT / 'apps/android/app/build/outputs/apk/androidTest/peercheck/app-peercheck-androidTest.apk'))
            adb('install', '-r', str(ROOT / 'apps/android/discovery-example/build/outputs/apk/debug/discovery-example-debug.apk'))
            adb('shell', 'pm', 'clear', PKG)
            adb('shell', 'run-as', PKG, 'mkdir', '-p', 'files')
            adb('shell', 'pm', 'grant', PKG, 'android.permission.POST_NOTIFICATIONS')
            invitation = root / 'invitation.json'
            issued = cli('host-invite', '--output', invitation)['invitation']
            adb('shell', '-T', shlex.join(['run-as', PKG, 'sh', '-c', 'umask 077; cat > files/discovery-invitation.json']), input=invitation.read_text())
            adb('shell', 'input', 'keyevent', '224')
            adb('shell', 'wm', 'dismiss-keyguard')
            log = root / 'instrumentation.log'
            with log.open('w') as output:
                process = subprocess.Popen(['adb', '-s', args.serial, 'shell', 'am', 'instrument', '-w', '-e', 'mode', 'discovery', '-e', 'host_a', host, '-e', 'fixture', service, PKG + '.test/com.biplabs.wispnet.AppTestRunner'], stdout=output, stderr=output, text=True)
                seen = set(); deadline = time.monotonic() + 180
                while process.poll() is None:
                    assert time.monotonic() < deadline, 'Discovery instrumentation timed out'
                    text = log.read_text()
                    for line in text.splitlines():
                        if 'stage=' not in line:
                            continue
                        stage = line.split('stage=', 1)[1].strip()
                        if stage in seen:
                            continue
                        seen.add(stage); print(stage, flush=True)
                        if stage == 'discovery-pending-owner':
                            pending = next(i for i in cli('host-invitations') if i['invitation'] == issued and i['state'] == 'pending')
                            peer = pending['peer'];mutate({'op': 'approve_invitation', 'invitation': issued})
                            mutate({'op': 'service', 'id': service, 'backend': f'127.0.0.1:{server.server_port}', 'kind': 'http', 'display_name': 'Discovery', 'allow': [peer]});registered.append(service)
                            mutate({'op': 'service', 'id': service + '-tcp', 'backend': f'127.0.0.1:{server.server_port}', 'kind': 'tcp', 'allow': [peer]});registered.append(service + '-tcp')
                            adb('shell', 'run-as', PKG, 'touch', 'files/discovery-continue')
                        elif stage == 'discovery-service-gear-revoked':
                            image = subprocess.check_output(['adb', '-s', args.serial, 'exec-out', 'run-as', PKG, 'cat', 'files/service-gears.png'])
                            Path('/tmp/wispnet-service-gears.png').write_bytes(image)
                            image = subprocess.check_output(['adb', '-s', args.serial, 'exec-out', 'run-as', PKG, 'cat', 'files/service-permissions.png'])
                            Path('/tmp/wispnet-service-permissions.png').write_bytes(image)
                        elif stage == 'discovery-withdraw-service':
                            mutate({'op': 'service', 'id': service, 'backend': f'127.0.0.1:{server.server_port}', 'kind': 'http', 'allow': []})
                            adb('shell', 'run-as', PKG, 'touch', 'files/discovery-continue')
                    time.sleep(.2)
            text = log.read_text()
            assert 'result=passed' in text, text
            assert 'discovery-external-http' in seen, text
            print('Independent app discovery, spoof denial, consent denial/approval, HTTP, revocation and host denial passed')
        finally:
            if process and process.poll() is None:
                adb('shell', 'am', 'force-stop', PKG);process.terminate();process.wait(timeout=10)
            cleanup_errors = []
            for entry in reversed(registered):
                try:
                    mutate({'op': 'remove', 'id': entry})
                except Exception as error:
                    cleanup_errors.append(str(error))
            if peer:
                try:
                    mutate({'op': 'revoke', 'peer': peer})
                except Exception as error:
                    cleanup_errors.append(str(error))
            if issued:
                try:
                    entry = next(i for i in cli('host-invitations') if i['invitation'] == issued)
                    if entry['state'] in ('issued', 'pending'):
                        mutate({'op': 'reject_invitation', 'invitation': issued})
                except StopIteration:
                    pass
                except Exception as error:
                    cleanup_errors.append(str(error))
            for pkg in [EXAMPLE, PKG + '.test', PKG]:
                try:
                    adb('uninstall', pkg)
                except subprocess.CalledProcessError:
                    pass
            server.shutdown();server.server_close()
            if cleanup_errors:
                raise RuntimeError('Fixture cleanup failed: ' + '; '.join(cleanup_errors))



if __name__ == '__main__':
    main()
