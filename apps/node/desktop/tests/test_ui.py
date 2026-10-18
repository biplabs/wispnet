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

    def test_stable_browser_url_needs_no_capability_or_handoff_file(self):
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
                    {"url": "http://w0123.localhost:1234/"},
                    None,
                )
            self.assertEqual(len(urls), 1)
            self.assertEqual(urls[0].toString(), "http://w0123.localhost:1234/")
            self.assertEqual(list(Path(directory).iterdir()), [])
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

    def test_persisted_catalog_is_stale_and_retains_high_water_after_restart(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(Window, "refresh", lambda self: None),
        ):
            api = API()
            api.state = Path(directory)
            peer = "a" * 64
            catalog = {
                "host": peer,
                "version": 1,
                "complete": True,
                "service_count": 1,
                "generation": "c" * 64,
                "revision": 5,
                "services": [
                    {
                        "id": "website",
                        "incarnation": "a" * 64,
                        "display_name": "Website",
                        "kind": "http",
                    }
                ],
            }
            first = Window(api, tray=False)
            first.timer.stop()
            first.peer_updated(peer, catalog, None)
            second = Window(api, tray=False)
            second.timer.stop()
            snapshot = {
                "access_enabled": True,
                "host_name": "Local",
                "connected_peers": {},
                "transport_streams": 0,
                "relay": "unavailable",
                "invitations": [],
                "peer_pins": [peer],
                "host": {"host": "f" * 64, "clients": [], "services": []},
            }
            calls = []
            second.submit = lambda fn, callback, peer=False: calls.append(fn)
            second.updated(snapshot, None)
            self.assertEqual(second.catalogs[peer]["catalog"], catalog)
            self.assertIn("Cached", second.peers.item(0, 1).text())
            calls.clear()
            second.open_peer(0, 0)
            self.assertEqual(calls, [])
            second.peer_updated(peer, {**catalog, "revision": 4}, None)
            self.assertIn("rejected", second.catalogs[peer]["state"])
            second.peer_updated(peer, {**catalog, "generation": "other"}, None)
            self.assertIn("rejected", second.catalogs[peer]["state"])
            second.peer_updated(peer, {**catalog, "services": []}, None)
            self.assertIn("rejected", second.catalogs[peer]["state"])
            second.peer_updated(peer, None, "access denied")
            self.assertEqual(second.catalogs[peer]["state"], "Denied")
            self.assertEqual(second.catalog_cache.load(peer)["catalog"], catalog)
            second.peer_updated(peer, catalog, None)
            self.assertEqual(second.catalogs[peer]["state"], "Available")
            for now in [1.0, 9_999_999_999.0]:
                with patch("wispnet_desktop.app.time.time", return_value=now):
                    second.peer_updated(peer, {**catalog, "revision": 4}, None)
                    self.assertEqual(second.catalogs[peer]["catalog"], catalog)
            removed = {**catalog, "revision": 6, "services": [], "service_count": 0}
            second.peer_updated(peer, removed, None)
            self.assertEqual(second.catalogs[peer]["catalog"], removed)
            self.assertEqual(second.catalog_cache.load(peer)["catalog"], removed)
            second.updated({**snapshot, "peer_pins": []}, None)
            self.assertNotIn(peer, second.catalogs)
            first.deleteLater()
            second.deleteLater()

    def test_local_open_uses_owner_host_and_requires_access(self):
        with patch.object(Window, "refresh", lambda self: None):
            window = Window(API(), tray=False)
            window.timer.stop()
            window.online = True
            window.snapshot = {
                "host": {
                    "host": "f" * 64,
                    "services": [
                        {
                            "id": "website",
                            "incarnation": "a" * 64,
                            "display_name": "Website",
                            "kind": "http",
                        }
                    ],
                }
            }
            window.fill(window.services, [["website", "127.0.0.1:1234", "Nobody"]])
            window.services.selectRow(0)
            calls = []
            window.submit = lambda fn, callback, peer=False: calls.append(fn)
            window.open_local()
            self.assertEqual(calls, [])
            window.access.setChecked(True)
            window.open_local()
            request = calls[-1]
            with patch.object(window.api, "request", return_value={}) as call:
                request()
                call.assert_called_once_with(
                    op="open", host="f" * 64, service="website", incarnation="a" * 64
                )
            window.deleteLater()

    def test_combined_view_never_calls_partial_results_complete(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(Window, "refresh", lambda self: None),
        ):
            api = API()
            api.state = Path(directory)
            api.config = Path(directory) / "host.toml"
            api.config.write_text(f'state_dir="{directory}"\n')
            window = Window(api, tray=False)
            window.timer.stop()
            states = {"a" * 64: "applied", "b" * 64: "pending", "c" * 64: "rejected"}

            def attempt(value, target):
                state = states[target["host"]]
                return {
                    "state": state,
                    "revision": 1 if state == "applied" else None,
                    "reason": state,
                }

            with patch.object(window.combined, "attempt", side_effect=attempt):
                value = window.combined.start(
                    {
                        "action": {"op": "revoke", "peer": "d" * 64},
                        "targets": [{"host": p} for p in states],
                    }
                )
            window.combined_result(value, None)
            window.tabs.setCurrentIndex(4)
            window.show()
            QApplication.processEvents()
            self.assertTrue(window.operations.isVisible())
            self.assertGreater(window.operations.height(), 100)
            self.assertEqual(window.operations.rowCount(), 3)
            self.assertEqual(
                {window.operations.item(i, 3).text() for i in range(3)},
                set(states.values()),
            )
            self.assertIn("0 fully acknowledged", window.operations_summary.text())
            self.assertIn("1 pending", window.operations_summary.text())
            self.assertIn("Not fully applied", window.message.text())
            window.deleteLater()

    def test_peer_catalog_origin_and_versions_remain_isolated(self):
        window = Window(API(), tray=False)
        window.timer.stop()
        peer = "a" * 64
        good = {
            "host": peer,
            "version": 1,
            "complete": True,
            "service_count": 1,
            "generation": "c" * 64,
            "revision": 2,
            "services": [
                {
                    "id": "website",
                    "incarnation": "a" * 64,
                    "display_name": "Website",
                    "kind": "http",
                }
            ],
        }
        window.peer_updated(peer, good, None)
        window.peer_updated(peer, {**good, "revision": 1}, None)
        self.assertEqual(window.catalogs[peer]["catalog"]["revision"], 2)
        self.assertIn("rejected", window.catalogs[peer]["state"])
        window.peer_updated(
            "b" * 64,
            {
                "host": "b" * 64,
                "version": 1,
                "complete": True,
                "service_count": 0,
                "generation": "d" * 64,
                "revision": 1,
                "services": [],
            },
            None,
        )
        self.assertEqual(window.catalogs[peer]["catalog"]["host"], peer)
        window.pool.waitForDone()
        QApplication.processEvents()
        window.deleteLater()


if __name__ == "__main__":
    unittest.main()
