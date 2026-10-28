import importlib.util
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location('install_command_center', Path(__file__).with_name('install_command_center.py'))
INSTALLER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INSTALLER)


class CredentialInputTests(unittest.TestCase):
    def test_protected_github_input_is_data_and_rejects_links(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'github.env'
            path.write_text("WISPNET_GITHUB_CLIENT_ID=fixture-id\nWISPNET_GITHUB_CLIENT_SECRET='literal-$(not-a-command)'\n")
            path.chmod(0o600)
            values = INSTALLER.read_environment(path)
            self.assertEqual(values['WISPNET_GITHUB_CLIENT_SECRET'], 'literal-$(not-a-command)')
            linked = path.with_name('linked.env')
            linked.symlink_to(path)
            with self.assertRaises(ValueError):
                INSTALLER.read_environment(linked)
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                INSTALLER.read_environment(path)

    def test_missing_and_extra_settings_fail_before_deployment(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'github.env'
            for text in ['WISPNET_GITHUB_CLIENT_ID=id\n', 'WISPNET_GITHUB_CLIENT_ID=id\nWISPNET_GITHUB_CLIENT_SECRET=secret\nOTHER=value\n',
                         'WISPNET_GITHUB_CLIENT_ID=id\nWISPNET_GITHUB_CLIENT_ID=other\nWISPNET_GITHUB_CLIENT_SECRET=secret\n']:
                path.write_text(text)
                path.chmod(0o600)
                with self.assertRaises(ValueError):
                    INSTALLER.read_environment(path)

    def test_hardlinks_and_fifos_are_rejected_without_reading(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'github.env'
            path.write_text('WISPNET_GITHUB_CLIENT_ID=id\nWISPNET_GITHUB_CLIENT_SECRET=secret\n')
            path.chmod(0o600)
            os.link(path, path.with_name('second.env'))
            with self.assertRaises(ValueError):
                INSTALLER.read_environment(path)
            path.unlink()
            os.mkfifo(path, 0o600)
            with self.assertRaises(ValueError):
                INSTALLER.read_environment(path)


def executable(path, machine, payload=b'fixture'):
    path.parent.mkdir(parents=True, exist_ok=True)
    header = b'\x7fELF' + bytes([2, 1, 1]) + bytes(9) + struct.pack('<HH', 3, machine)
    path.write_bytes(header + payload)
    path.chmod(0o700)
    return path


class BinarySelectionTests(unittest.TestCase):
    def test_binary_architecture_is_checked_before_upload_or_build(self):
        with tempfile.TemporaryDirectory() as directory:
            path = executable(Path(directory) / 'misleading-arm64-name', 62)
            self.assertEqual(INSTALLER.binary_architecture(path), 'x86_64')
            with mock.patch.object(INSTALLER.subprocess, 'run') as run:
                with self.assertRaisesRegex(ValueError, 'does not match'):
                    INSTALLER.select_binary('aarch64', path)
                run.assert_not_called()
            path.write_text('#!/bin/sh\nexit 0\n')
            with self.assertRaisesRegex(ValueError, 'ELF'):
                INSTALLER.binary_architecture(path)

    def test_cross_architecture_needs_explicit_binary(self):
        with mock.patch.object(INSTALLER.platform, 'system', return_value='Linux'), \
             mock.patch.object(INSTALLER.platform, 'machine', return_value='x86_64'), \
             mock.patch.object(INSTALLER.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'requires --binary'):
                INSTALLER.select_binary('aarch64')
            run.assert_not_called()

    def test_native_build_is_locked_and_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def build(*args, **kwargs):
                executable(root / 'target/release/wispnet-command-center', 62)
            with mock.patch.object(INSTALLER, 'ROOT', root), \
                 mock.patch.object(INSTALLER.platform, 'system', return_value='Linux'), \
                 mock.patch.object(INSTALLER.platform, 'machine', return_value='x86_64'), \
                 mock.patch.object(INSTALLER.subprocess, 'run', side_effect=build) as run:
                self.assertEqual(INSTALLER.select_binary('x86_64'), root / 'target/release/wispnet-command-center')
                run.assert_called_once_with(['cargo', 'build', '--release', '--locked', '-p', 'wispnet-command-center'], cwd=root, check=True)


class FakeRemote:
    """Execute deployment file operations locally; fake only SSH and systemd."""
    def __init__(self, root, healthy=True, existing=True):
        self.home = root / 'remote home'
        self.home.mkdir()
        self.app = self.home / '.local/libexec/wispnet-command-center'
        self.config = self.home / '.config/wispnet-command-center'
        self.data = self.home / '.local/share/wispnet-command-center/state'
        self.unit = self.home / '.config/systemd/user' / INSTALLER.SERVICE
        self.bin = root / 'fake-bin'
        self.bin.mkdir()
        self.runtime = root / 'systemd-state'
        self.runtime.mkdir()
        self.healthy = healthy
        self.scripts = []
        for path in [self.app, self.config, self.data, self.unit.parent]:
            path.mkdir(parents=True, exist_ok=True)
        (self.data / 'database-sentinel').write_text('persistent account state')
        (self.home / 'live-caddy-config').write_text('existing HTTPS routes')
        if existing:
            executable(self.app / 'wispnet-command-center', 62, b'old executable')
            (self.config / 'command.env').write_text('old protected environment\n')
            (self.config / 'command.env').chmod(0o600)
            self.unit.write_text('old service definition\n')
            (self.runtime / 'active').touch()
            (self.runtime / 'enabled').touch()
        shim = self.bin / 'systemctl'
        shim.write_text('''#!/bin/sh
set -eu
[ "$1" = --user ]
shift
case "$1" in
is-active) test -e "$FAKE_SYSTEMD/active" ;;
is-enabled) test -e "$FAKE_SYSTEMD/enabled" ;;
enable) touch "$FAKE_SYSTEMD/enabled" ;;
disable) rm -f "$FAKE_SYSTEMD/enabled" ;;
restart|start) touch "$FAKE_SYSTEMD/active" ;;
stop) rm -f "$FAKE_SYSTEMD/active" ;;
daemon-reload) true ;;
*) exit 1 ;;
esac
''')
        shim.chmod(0o700)

    def run(self, script):
        self.scripts.append(script)
        if script == 'printf "%s" "$HOME"':
            return str(self.home)
        if script == 'uname -m':
            return 'x86_64'
        if script.startswith('# wispnet-command-health-check'):
            if not self.healthy:
                raise RuntimeError('fixture health failure')
            return ''
        environment = dict(os.environ, HOME=str(self.home), FAKE_SYSTEMD=str(self.runtime),
                           PATH=str(self.bin) + os.pathsep + os.environ['PATH'])
        return subprocess.run(['sh', '-c', script], env=environment, capture_output=True,
                              text=True, check=True).stdout.strip()

    def put(self, source, destination):
        shutil.copy2(source, destination)


class DeploymentTests(unittest.TestCase):
    github = {'WISPNET_GITHUB_CLIENT_ID': 'fixture-id', 'WISPNET_GITHUB_CLIENT_SECRET': 'fixture-secret'}

    def deploy(self, remote, root):
        binary = executable(root / 'new-binary', 62, b'new executable')
        INSTALLER.deploy(remote, binary, self.github, 'https://cmd.wispnet.app', 'wispnet.app')
        return binary

    def assert_preserved(self, remote):
        self.assertEqual((remote.data / 'database-sentinel').read_text(), 'persistent account state')
        self.assertEqual((remote.home / 'live-caddy-config').read_text(), 'existing HTTPS routes')
        self.assertEqual(list(remote.app.glob('.install.*')), [])

    def test_stages_one_rust_binary_protects_credentials_and_preserves_state(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            remote = FakeRemote(root)
            binary = self.deploy(remote, root)
            installed = remote.app / 'wispnet-command-center'
            self.assertEqual(installed.read_bytes(), binary.read_bytes())
            self.assertEqual(installed.stat().st_mode & 0o777, 0o700)
            self.assertEqual((remote.config / 'command.env').stat().st_mode & 0o777, 0o600)
            self.assertIn('WISPNET_GITHUB_CLIENT_SECRET="fixture-secret"', (remote.config / 'command.env').read_text())
            self.assertNotIn('WISPNET_COMMAND_SECRET', (remote.config / 'command.env').read_text())
            self.assertNotIn('node', remote.unit.read_text().lower())
            self.assertTrue((remote.runtime / 'active').exists())
            self.assertTrue((remote.runtime / 'enabled').exists())
            self.assertFalse(any('npm' in script or 'nodejs.org' in script for script in remote.scripts))
            self.assert_preserved(remote)

    def test_health_failure_restores_executable_environment_and_unit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            remote = FakeRemote(root, healthy=False)
            original = [(path, path.read_bytes()) for path in [remote.app / 'wispnet-command-center', remote.config / 'command.env', remote.unit]]
            with self.assertRaisesRegex(RuntimeError, 'restored'):
                self.deploy(remote, root)
            for path, content in original:
                self.assertEqual(path.read_bytes(), content)
            self.assertTrue((remote.runtime / 'active').exists())
            self.assertTrue((remote.runtime / 'enabled').exists())
            self.assert_preserved(remote)

    def test_failed_first_install_leaves_no_enabled_service_or_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            remote = FakeRemote(root, healthy=False, existing=False)
            with self.assertRaisesRegex(RuntimeError, 'restored'):
                self.deploy(remote, root)
            for path in [remote.app / 'wispnet-command-center', remote.config / 'command.env', remote.unit, remote.runtime / 'active', remote.runtime / 'enabled']:
                self.assertFalse(path.exists())
            self.assert_preserved(remote)


if __name__ == '__main__':
    unittest.main()
