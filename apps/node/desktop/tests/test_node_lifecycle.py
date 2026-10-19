import subprocess
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from wispnet_desktop.api import NodeAPI


class NodeLifecycleChecks(unittest.TestCase):
    def api(self):
        api = NodeAPI.__new__(NodeAPI)
        api.config = (Path.home() / ".config/wipsnet-node/host.toml").resolve()
        api.request = Mock(return_value={"api_version": 1})
        return api

    def test_stop_and_start_only_control_host_unit_and_wait_for_api(self):
        api = self.api()
        with patch("subprocess.run", return_value=subprocess.CompletedProcess([], 0)) as run:
            self.assertIsNone(api.set_online(False))
            api.request.assert_not_called()
            self.assertEqual(run.call_args.args[0], [
                "systemctl", "--user", "stop", "wipsnet-host.service",
            ])
            api.request.side_effect = [OSError("starting"), {"api_version": 1}]
            with patch("time.sleep"):
                self.assertEqual(api.set_online(True), {"api_version": 1})
            self.assertEqual(run.call_args.args[0], [
                "systemctl", "--user", "start", "wipsnet-host.service",
            ])
            self.assertEqual(api.request.call_count, 2)

    def test_failures_are_reported_and_custom_config_never_controls_default_node(self):
        api = self.api()
        with patch("subprocess.run", return_value=subprocess.CompletedProcess([], 1, stderr="failed")) as run:
            with self.assertRaisesRegex(ValueError, "failed"):
                api.set_online(False)
            api.config = Path("/tmp/other-node.toml")
            run.reset_mock()
            with self.assertRaisesRegex(ValueError, "configuration"):
                api.set_online(False)
            run.assert_not_called()
