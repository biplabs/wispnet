import unittest
import tempfile
from pathlib import Path
from unittest.mock import patch
from PySide6.QtWidgets import QApplication
from PySide6.QtCore import Qt
from wispnet_desktop.app import Window, appearance


class API:
    def request(self, **request):
        raise OSError("fixture unavailable")


class DesktopChecks(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])
        appearance(cls.app)

    def test_identity_selection_and_failure_do_not_invent_connected_clients(self):
        window = Window(API(), tray=False)
        window.timer.stop()
        window.updated(
            {
                "api_version": 1,
                "access_enabled": False,
                "host_name": "Local",
                "connected_peers": {"a" * 64: 2},
                "recent_peers": {"a" * 64: 1, "b" * 64: 12},
                "presence_timeout_seconds": 60,
                "transport_streams": 1,
                "relay": "connected",
                "invitations": [],
                "peer_pins": [],
                "host": {
                    "host": "f" * 64,
                    "revision": 1,
                    "clients": [
                        {"peer": "a" * 64, "name": "Same"},
                        {"peer": "b" * 64, "name": "Same"},
                    ],
                    "services": [],
                },
            },
            None,
        )
        self.assertIn("Active clients: 2", window.status.text())
        self.assertEqual(window.devices.item(0, 4).text(), "Connected")
        self.assertEqual(window.devices.item(1, 4).text(), "Seen 12s ago")
        # Expired recent activity does not hide an actually open connection.
        snapshot = dict(window.snapshot)
        snapshot["recent_peers"] = {}
        window.updated(snapshot, None)
        self.assertIn("Active clients: 1", window.status.text())
        self.assertEqual(window.devices.item(1, 4).text(), "Idle")
        window.devices.selectRow(1)
        window.fill(
            window.devices,
            [["Same", "b" * 64, "Approved", 0], ["Same", "a" * 64, "Approved", 2]],
        )
        self.assertEqual(
            window.devices.item(window.devices.currentRow(), 1).text(), "b" * 64
        )
        window.updated(None, "offline")
        self.assertFalse(window.online)
        self.assertEqual(window.devices.rowCount(), 0)
        self.assertFalse(window.access.isEnabled())
        self.assertIn("unavailable", window.status.text())
        window.pool.waitForDone()
        QApplication.processEvents()
        window.deleteLater()

    def test_browser_arguments_do_not_contain_bootstrap_capability(self):
        with tempfile.TemporaryDirectory() as directory:
            api = API()
            api.state = Path(directory)
            window = Window(api, tray=False)
            window.timer.stop()
            urls = []
            with patch(
                "wispnet_desktop.app.QDesktopServices.openUrl",
                side_effect=lambda url: urls.append(url) or True,
            ):
                window.open_result(
                    {"url": "http://w0123.localhost:1234/_wipsnet/start#private-token"},
                    None,
                )
            self.assertEqual(len(urls), 1)
            self.assertEqual(urls[0].scheme(), "file")
            self.assertNotIn("private-token", urls[0].toString())
            file = Path(urls[0].toLocalFile())
            self.assertEqual(file.stat().st_mode & 0o777, 0o600)
            self.assertIn("private-token", file.read_text())
            window.pool.waitForDone()
            QApplication.processEvents()
            window.deleteLater()

    def test_pair_name_is_private_display_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            api = API()
            api.state = Path(directory)
            window = Window(api, tray=False)
            window.timer.stop()
            window.pair_saved(
                {"host": "a" * 64, "host_name": "Desktop"},
                {"state": "pending_owner_approval"},
                None,
            )
            file = Path(directory) / "desktop-peer-names.json"
            self.assertEqual(file.stat().st_mode & 0o777, 0o600)
            self.assertEqual(window.peer_names["a" * 64], "Desktop")
            self.assertNotIn("invitation", file.read_text())
            window.pool.waitForDone()
            QApplication.processEvents()
            window.deleteLater()

    def test_peer_catalog_origin_and_versions_remain_isolated(self):
        window = Window(API(), tray=False)
        window.timer.stop()
        peer = "a" * 64
        good = {
            "host": peer,
            "generation": "g",
            "revision": 2,
            "services": [{"id": "website"}],
        }
        window.peer_updated(peer, good, None)
        window.peer_updated(peer, {**good, "revision": 1}, None)
        self.assertEqual(window.catalogs[peer]["catalog"]["revision"], 2)
        self.assertIn("rejected", window.catalogs[peer]["state"])
        window.peer_updated(
            "b" * 64,
            {"host": "b" * 64, "generation": "z", "revision": 1, "services": []},
            None,
        )
        self.assertEqual(window.catalogs[peer]["catalog"]["host"], peer)
        window.pool.waitForDone()
        QApplication.processEvents()
        window.deleteLater()


if __name__ == "__main__":
    unittest.main()
