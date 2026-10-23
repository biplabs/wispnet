#!/usr/bin/env python3
"""Run the Flutter device checks using upgrade-only installs; restore the normal UI.

Requires an already installed debug app. Never uninstalls or clears application
state. Native host identity and saved host IDs must survive the test. Legacy profile
names may migrate to the equivalent canonical readable ID.
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
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = 'app.wispnet.android'
ACTIVITY = PACKAGE + '/com.biplabs.wispnet.FlutterMainActivity'


def readable_id(value):
    if re.fullmatch(r'[a-z0-9][a-z0-9._-]{0,63}', value):
        return value
    result = ''
    for c in value.lower():
        if c.isascii() and (c.isalnum() or result and c in '._-'):
            result += c
        elif result and not result.endswith('-'):
            result += '-'
        if len(result) == 64:
            break
    return result.rstrip('._-') or 'device'


def check_network_process(adb, private):
    """Exercise a real scheme handoff, then kill only the Flutter process."""
    prefs = ET.fromstring(private('shared_prefs/hosts.xml'))
    records = json.loads(next(entry.text for entry in prefs if entry.get('name') == 'records'))
    selected = next(((host, service['id']) for host, row in records.items()
                     for service in row.get('catalog', {}).get('services', [])
                     if service.get('kind') == 'http'), None)
    if selected is None:
        raise RuntimeError('A paired HTTP service is required for the network-process check')
    host, service = selected
    link = f'wispnet://{host}/{service}'
    adb('shell', 'am', 'start', '-a', 'android.intent.action.VIEW', '-d', link, '-n', ACTIVITY)
    forwarded = None
    try:
        try:
            port = json.loads(private('files/client/access-settings.json'))['registry_port']
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            port = 11840
        forwarded = adb('forward', 'tcp:0', 'tcp:' + str(port), capture_output=True, text=True).stdout.strip()
        def request(path, authority):
            with urlopen(Request(f'http://127.0.0.1:{forwarded}{path}', headers={'Host': authority}), timeout=20) as response:
                return response.read()
        for _ in range(60):
            try:
                directory = json.loads(request('/v1/services', f'localhost:{port}'))
                route = next(row for row in directory['services'] if row['host_id'] == host and row['service_id'] == service and row.get('url'))
                break
            except (OSError, ValueError, StopIteration):
                time.sleep(.25)
        else:
            raise RuntimeError('Deep link did not bring the shared listener online')
        from urllib.parse import urlsplit
        endpoint = urlsplit(route['url'])
        assert endpoint.port == port
        assert request('/', endpoint.netloc), 'Service did not return a response through P2P'
        ui_pid = adb('shell', 'pidof', PACKAGE, capture_output=True, text=True).stdout.strip()
        network_pid = adb('shell', 'pidof', PACKAGE + ':network', capture_output=True, text=True).stdout.strip()
        assert ui_pid and network_pid and ui_pid != network_pid
        adb('shell', 'run-as', PACKAGE, 'kill', '-9', ui_pid)
        time.sleep(.5)
        assert adb('shell', 'pidof', PACKAGE + ':network', capture_output=True, text=True).stdout.strip() == network_pid
        assert json.loads(request('/v1/services', f'localhost:{port}'))['api_version'] == 1
        assert request('/', endpoint.netloc), 'P2P service stopped when the UI process died'
        print('PASS: deep-link handoff and P2P API survive Flutter UI process death')
    finally:
        if forwarded:
            adb('forward', '--remove', 'tcp:' + forwarded)


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
        profile = {entry.get('name'): entry.text for entry in ET.fromstring(private('shared_prefs/device_profile.xml'))}
        return {
            'identity': hashlib.sha256(private('shared_prefs/identity.xml')).hexdigest(),
            'device_id': readable_id(profile.get('device_id') or profile.get('name') or 'android-device'),
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
            for name in ['directory', 'machines', 'settings', 'offline-control', 'standby']:
                (args.output / ('device-' + name + '.png')).write_bytes(private('cache/flutter-device-' + name + '.png'))
            assert preserved() == before, 'Upgrade changed identity, device profile or saved host IDs'
            print('PASS: Flutter installed-app checks and identity/host preservation')
            adb('install', '-r', str(original))
            check_network_process(adb, private)
        finally:
            if forwarded:
                adb('forward', '--remove', 'tcp:' + forwarded)
            # adb -r fails safely rather than uninstalling on signature/version errors.
            adb('install', '-r', str(original))
            adb('shell', 'am', 'start', '-n', ACTIVITY)
            assert preserved() == before, 'Normal UI restore changed preserved app state'


if __name__ == '__main__':
    main()
