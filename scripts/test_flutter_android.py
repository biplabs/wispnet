#!/usr/bin/env python3
"""Run the Flutter device checks using upgrade-only installs; restore the normal UI.

Requires an already installed debug app. Never uninstalls or clears application
state. Native identity/profile hashes and saved host IDs must survive the test.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = 'app.wispnet.android'
ACTIVITY = PACKAGE + '/com.biplabs.wispnet.FlutterMainActivity'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--output', type=Path, default=Path('/tmp/wispnet-flutter'))
    args = parser.parse_args()
    flutter = ROOT / 'scripts/flutter.sh'
    def run(command, **kwargs):
        return subprocess.run(command, check=True, **kwargs)
    def adb(*command, **kwargs):
        return run(['adb', '-s', args.serial, *command], **kwargs)
    def private(path):
        return adb('exec-out', 'run-as', PACKAGE, 'cat', path, capture_output=True).stdout
    def preserved():
        hosts = ET.fromstring(private('shared_prefs/hosts.xml'))
        records = json.loads(next(entry.text for entry in hosts if entry.get('name') == 'records'))
        return {
            'identity': hashlib.sha256(private('shared_prefs/identity.xml')).hexdigest(),
            'profile': hashlib.sha256(private('shared_prefs/device_profile.xml')).hexdigest(),
            'hosts': set(records),
        }
    before = preserved()
    args.output.mkdir(parents=True, exist_ok=True)
    forwarded = None
    with tempfile.TemporaryDirectory(prefix='wispnet-flutter-device-') as temporary:
        original = Path(temporary) / 'normal.apk'
        run([str(ROOT / 'scripts/build_android.sh')], cwd=ROOT)
        source = ROOT / 'apps/flutter/build/app/outputs/flutter-apk/app-debug.apk'
        original.write_bytes(source.read_bytes())
        try:
            run([str(flutter), 'build', 'apk', '--debug', '--target-platform', 'android-arm64',
                 '--target', 'integration_test/android_upgrade_test.dart'], cwd=ROOT / 'apps/flutter')
            adb('install', '-r', str(source))
            adb('shell', 'am', 'start', '-n', ACTIVITY)
            for _ in range(100):
                try:
                    pid = adb('shell', 'pidof', PACKAGE, capture_output=True, text=True).stdout.strip()
                except subprocess.CalledProcessError:
                    time.sleep(.2)
                    continue
                logs = adb('logcat', '-d', '--pid=' + pid, '-s', 'flutter', capture_output=True, text=True).stdout
                matches = re.findall(r'The Dart VM service is listening on (http://127.0.0.1:(\d+)/\S+)', logs)
                if matches:
                    url, remote = matches[-1]
                    forwarded = adb('forward', 'tcp:0', 'tcp:' + remote, capture_output=True, text=True).stdout.strip()
                    url = url.replace(':' + remote + '/', ':' + forwarded + '/')
                    break
                time.sleep(.2)
            else:
                raise RuntimeError('Installed Flutter test did not start')
            run([str(flutter), 'drive', '--use-existing-app=' + url,
                 '--driver=test_driver/integration_test.dart'], cwd=ROOT / 'apps/flutter', timeout=180)
            for name in ['directory', 'machines', 'settings']:
                (args.output / ('device-' + name + '.png')).write_bytes(private('cache/flutter-device-' + name + '.png'))
            assert preserved() == before, 'Upgrade changed identity, device profile or saved host IDs'
            print('PASS: Flutter installed-app checks and identity/host preservation')
        finally:
            if forwarded:
                adb('forward', '--remove', 'tcp:' + forwarded)
            # adb -r fails safely rather than uninstalling on signature/version errors.
            adb('install', '-r', str(original))
            adb('shell', 'am', 'start', '-n', ACTIVITY)
            assert preserved() == before, 'Normal UI restore changed preserved app state'


if __name__ == '__main__':
    main()
