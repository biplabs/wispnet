#!/usr/bin/env python3
"""Exercise Android Files through private Binder/JNI against an isolated host.

Run on a disposable emulator with the freshly built debug and androidTest APKs.
The fixture creates its own identity, receiving folder and grants; installed
desktop nodes and physical Android devices are never changed.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'apps/node/desktop'))
from wispnet_desktop.api import NodeAPI


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--apk', type=Path, default=ROOT / 'apps/flutter/build/app/outputs/flutter-apk/app-debug.apk')
    parser.add_argument('--test-apk', type=Path, default=ROOT / 'apps/flutter/build/app/outputs/apk/androidTest/debug/app-debug-androidTest.apk')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)

    def adb(*command, **kwargs):
        return subprocess.run(['adb', '-s', args.serial, *map(str, command)],
                              check=True, capture_output=True, **kwargs)

    if adb('shell', 'getprop', 'ro.kernel.qemu', text=True).stdout.strip() != '1':
        raise SystemExit('This fixture runs only on a disposable emulator.')
    for apk in [args.apk, args.test_apk]:
        if not apk.is_file():
            raise SystemExit(f'Build the current APK first: {apk}')
        adb('install', '-r', apk)
    adb('shell', 'pm', 'grant', 'app.wispnet.android', 'android.permission.POST_NOTIFICATIONS')
    binary = ROOT / 'target/debug/wispnet'
    with tempfile.TemporaryDirectory(prefix='wispnet-android-files-') as directory:
        temporary = Path(directory)
        export = temporary / 'incoming'
        export.mkdir(mode=0o700)
        config = temporary / 'host.toml'
        config.write_text(
            f'schema_version=2\ndevice_name="files-test-host"\nstate_dir="{temporary / "state"}"\n'
            'log_level="info"\n[transport]\nbind_addr="127.0.0.1:0"\n'
            'relay_url="https://relay.wispnet.app"\n')
        subprocess.run([binary, 'host-init', '--config', config], check=True, capture_output=True)
        with (args.output / 'host.log').open('w') as log:
            process = subprocess.Popen([binary, 'agent', '--config', config], stdout=subprocess.DEVNULL,
                                       stderr=subprocess.PIPE, text=True)
            stopped = threading.Event()
            errors = []
            watcher = None
            try:
                line = process.stderr.readline()
                started = json.loads(line)['fields']
                if started['event'] != 'started':
                    raise RuntimeError(f'Fixture did not start: {line}')
                port = int(started['bind_addr'].rsplit(':', 1)[1])
                log.write(line)

                def drain():
                    for line in process.stderr:
                        log.write(line)
                drainer = threading.Thread(target=drain, daemon=True)
                drainer.start()
                api = NodeAPI(config)

                def mutate(**operation):
                    operation['expected_revision'] = api.request(op='status')['host']['revision']
                    return api.request(op='mutate', operation=operation)

                status = api.request(op='status')
                host = status['host']['host']
                mutate(op='files', exports={'training': str(export)}, allow=[])
                invitation_file = temporary / 'invitation.json'
                subprocess.run([binary, 'host-invite', '--config', config, '--output', invitation_file],
                               check=True, capture_output=True)
                invitation = json.loads(invitation_file.read_text())
                invitation['direct_hint'] = f'10.0.2.2:{port}'
                encoded = 'wispnet1:' + base64.urlsafe_b64encode(json.dumps(invitation).encode()).decode().rstrip('=')

                def approve():
                    try:
                        while not stopped.wait(.1):
                            status = api.request(op='status')
                            for request in status['invitations']:
                                if request['state'] == 'pending':
                                    mutate(op='approve_invitation', invitation=request['invitation'])
                                    mutate(op='grant_service', service='files', peer=request['peer'])
                    except Exception as error:
                        errors.append(str(error))

                watcher = threading.Thread(target=approve, daemon=True)
                watcher.start()
                command = ['am', 'instrument', '-w', '-r', '-e', 'host', host,
                           '-e', 'invitation', encoded, '-e', 'workspace', 'training',
                           'app.wispnet.android.test/com.biplabs.wispnet.FilesTransferInstrumentation']
                result = adb('shell', ' '.join(map(shlex.quote, command)), text=True, timeout=240)
                (args.output / 'instrumentation.log').write_text(result.stdout + result.stderr)
                if errors:
                    raise RuntimeError(f'Fixture approval failed: {errors}')
                if 'INSTRUMENTATION_CODE: -1' not in result.stdout:
                    raise RuntimeError('Android integration failed; inspect instrumentation.log')
                raw = adb('exec-out', 'run-as', 'app.wispnet.android', 'cat',
                          'cache/android-files-result.json').stdout
                receipt = json.loads(raw)
                if not receipt.get('success'):
                    raise RuntimeError(f'Android integration failed: {receipt}')
                verified = []
                for file in receipt['files']:
                    destination = export / file['path']
                    if not destination.is_file() or destination.is_symlink():
                        raise AssertionError(f'Expected a completed regular file: {file["path"]}')
                    with destination.open('rb') as content:
                        digest = hashlib.file_digest(content, 'sha256').hexdigest()
                    if digest != file['sha256'] or destination.stat().st_size != file['size_bytes']:
                        raise AssertionError(f'Destination hash/size mismatch: {file["path"]}')
                    verified.append(file['path'])
                receipt['host_verified_destinations'] = verified
                receipt['isolated_host_only'] = True
                (args.output / 'native-verification.json').write_text(json.dumps(receipt, indent=2) + '\n')
                print(json.dumps({'native_android_files_passed': True, 'verified_files': verified}))
            finally:
                stopped.set()
                if watcher:
                    watcher.join(timeout=3)
                process.terminate()
                try:
                    process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


if __name__ == '__main__':
    main()
