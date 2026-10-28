import unittest
from unittest.mock import Mock, patch

import command_center


class CommandCenterTests(unittest.TestCase):
    def setUp(self):
        self.status = {
            'base_url': 'https://cmd.wispnet.app',
            'pending': {'verification_url': 'https://cmd.wispnet.app/connect?code=ABCD-EFGH'},
        }
        self.api = Mock()
        self.api.request.return_value = self.status

    def test_browser_handoff_uses_node_state_and_ignores_supplied_url(self):
        with patch.object(command_center.subprocess, 'run') as opened:
            command_center.dispatch('launchCommandCenter', {
                'purpose': 'link', 'url': 'https://attacker.example/'}, self.api)
            self.assertEqual(opened.call_args.args[0][-1], self.status['pending']['verification_url'])
            self.api.request.assert_called_once_with(op='command_center_status')

    def test_untrusted_or_secret_bearing_signin_urls_are_rejected(self):
        for url in [
            'https://attacker.example/connect?code=ABCD',
            'https://cmd.wispnet.app@attacker.example/connect?code=ABCD',
            'https://cmd.wispnet.app/connect?code=ABCD&device_code=secret',
            'https://cmd.wispnet.app/connect?code=one&code=two',
            'https://cmd.wispnet.app/redirect?code=ABCD',
            'javascript:alert(1)',
        ]:
            with self.subTest(url=url):
                self.status['pending']['verification_url'] = url
                with self.assertRaises(ValueError):
                    command_center.browser_target(self.status, 'link')

    def test_module_only_forwards_its_explicit_owner_operations(self):
        payload = {'op': 'configure_backend_auth', 'service': 'gike', 'mode': 'bearer', 'token': 'fixture'}
        command_center.dispatch('desktopRequest', {'payload': payload}, self.api)
        self.api.request.assert_called_once_with(**payload)
        self.api.request.reset_mock()
        self.assertIs(command_center.dispatch('desktopRequest', {'payload': {'op': 'mutate'}}, self.api), command_center.NOT_HANDLED)
        self.api.request.assert_not_called()

    def test_dashboard_has_no_pending_code_or_caller_controlled_path(self):
        self.assertEqual(command_center.browser_target(self.status, 'dashboard'), 'https://cmd.wispnet.app/')
        for base in ['https://cmd.wispnet.app/other', 'https://cmd.wispnet.app/?token=secret', 'http://cmd.wispnet.app', 'https://user:pass@cmd.wispnet.app']:
            with self.subTest(base=base), self.assertRaises(ValueError):
                command_center.browser_target(dict(self.status, base_url=base), 'dashboard')


if __name__ == '__main__':
    unittest.main()
