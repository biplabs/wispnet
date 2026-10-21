import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location(
    "desktop_data", ROOT / "apps/macos/desktop_data.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DesktopDataTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.config = self.root / "host.toml"
        self.state = self.root / "state"
        self.state.mkdir()
        self.config.write_text("state_dir=" + json.dumps(str(self.state)))

    def call(self, **payload):
        return module.dispatch(payload, self.config)

    def test_cache_is_display_only_validated_and_removed_on_unpair(self):
        host = "a" * 64
        catalog = dict(
            version=1,
            complete=True,
            service_count=0,
            host=host,
            generation="b" * 64,
            revision=1,
            services=[],
        )
        self.call(op="cache_save", host=host, catalog=catalog)
        result = self.call(op="cache_load", host=host)
        self.assertEqual(result["state"], "Cached; refresh required")
        self.assertEqual(result["catalog"], catalog)
        self.assertEqual(
            (self.state / "desktop-catalogs" / (host + ".json")).stat().st_mode & 0o077,
            0,
        )
        self.call(op="cache_remove", host=host)
        self.assertIsNone(self.call(op="cache_load", host=host))
        with self.assertRaises(ValueError):
            self.call(op="cache_save", host=host, catalog={**catalog, "host": "c" * 64})

    def test_reuses_durable_operation_journal_and_receipts(self):
        host, peer = "a" * 64, "b" * 64
        with patch.object(
            module.NodeAPI,
            "request",
            side_effect=lambda **r: dict(
                host=host,
                id=r["id"],
                state="applied",
                revision=4,
                reason="acknowledged",
            ),
        ) as request:
            operation = self.call(
                op="start",
                plan={
                    "action": {"op": "revoke", "peer": peer},
                    "targets": [{"host": host}],
                },
            )
            self.assertTrue(operation["complete"])
            self.assertEqual(self.call(op="operations"), [operation])
            self.assertEqual(self.call(op="retry", id=operation["id"]), operation)
            self.assertEqual(request.call_count, 1)
        with self.assertRaises(ValueError):
            self.call(
                op="start",
                plan={
                    "action": {"op": "approve", "peer": peer},
                    "targets": [{"host": host, "ssh": "-oProxyCommand=bad"}],
                },
            )

    def test_migrates_qt_names_and_order_without_qt_runtime(self):
        names = self.state / "desktop-peer-names.json"
        names.write_text(json.dumps({"a" * 64: "My host"}))
        names.chmod(0o600)
        conf = self.root / "wispnet/desktop-order.conf"
        conf.parent.mkdir()
        conf.write_text("[General]\nmachines=b, a\n[services]\na=web, llama\n")
        with patch.dict(os.environ, XDG_CONFIG_HOME=str(self.root)):
            result = self.call(op="preferences")
        self.assertEqual(result["names"], {"a" * 64: "My host"})
        self.assertEqual(
            result["order"], {"machines": ["b", "a"], "services/a": ["web", "llama"]}
        )

    def test_rejects_unknown_operations_and_invalid_target_configuration(self):
        with self.assertRaises(ValueError):
            self.call(op="shell", command="echo bad")
        targets = self.config.parent / "admin-targets.json"
        targets.write_text('[{"host":"bad","ssh":"bad"}]')
        targets.chmod(0o600)
        with self.assertRaises(ValueError):
            self.call(op="targets")


if __name__ == "__main__":
    unittest.main()
