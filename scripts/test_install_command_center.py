import importlib.util
from pathlib import Path
import tempfile
import unittest

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
            path.chmod(0o600) if path.exists() else None
            for text in ['WISPNET_GITHUB_CLIENT_ID=id\n', 'WISPNET_GITHUB_CLIENT_ID=id\nWISPNET_GITHUB_CLIENT_SECRET=secret\nOTHER=value\n']:
                path.write_text(text)
                path.chmod(0o600)
                with self.assertRaises(ValueError):
                    INSTALLER.read_environment(path)


if __name__ == '__main__':
    unittest.main()
