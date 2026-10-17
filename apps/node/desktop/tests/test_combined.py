import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from wispnet_desktop.combined import Combined, validate_plan
from wispnet_desktop.api import NodeRejected


class CombinedChecks(unittest.TestCase):
    def manager(self, root):
        config = root / "host.toml"
        config.write_text(f'state_dir="{root}"\n')
        return Combined(config)

    def test_partial_receipts_persist_and_retry_only_pending(self):
        with tempfile.TemporaryDirectory() as directory:
            manager = self.manager(Path(directory))
            a, b, c = "a" * 64, "b" * 64, "c" * 64
            calls = []

            def attempt(value, target):
                calls.append(target["host"])
                if target["host"] == b:
                    return {"state": "pending", "revision": None, "reason": "offline"}
                return {
                    "state": "rejected" if target["host"] == c else "applied",
                    "revision": None if target["host"] == c else 2,
                    "reason": "denied" if target["host"] == c else None,
                }

            with patch.object(manager, "attempt", side_effect=attempt):
                value = manager.start(
                    {
                        "action": {"op": "revoke", "peer": "d" * 64},
                        "targets": [{"host": p} for p in [a, b, c]],
                    }
                )
            self.assertFalse(value["complete"])
            self.assertFalse(value["finished"])
            self.assertEqual(manager.path(value["id"]).stat().st_mode & 0o777, 0o600)
            reloaded = self.manager(Path(directory))
            with patch.object(
                reloaded,
                "attempt",
                return_value={"state": "applied", "revision": 3, "reason": None},
            ) as retry:
                result = reloaded.retry(value["id"])
                self.assertEqual(retry.call_count, 1)
                self.assertEqual(retry.call_args.args[1]["host"], b)
            self.assertTrue(result["finished"])
            self.assertFalse(result["complete"])
            self.assertEqual(
                [t["state"] for t in result["targets"]],
                ["applied", "applied", "rejected"],
            )

    def test_unknown_reply_stays_pending_and_explicit_denial_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            manager = self.manager(Path(directory))
            value = {"id": "a" * 64, "action": {"op": "revoke", "peer": "b" * 64}}
            target = {"host": "c" * 64}
            with patch.object(manager.api, "request", side_effect=OSError("offline")):
                self.assertEqual(manager.attempt(value, target)["state"], "pending")
            with patch.object(
                manager.api, "request", side_effect=NodeRejected("Wrong target host")
            ):
                self.assertEqual(manager.attempt(value, target)["state"], "rejected")
            with patch.object(
                manager.api,
                "request",
                return_value={"state": "applied", "host": "d" * 64, "id": value["id"]},
            ):
                self.assertEqual(manager.attempt(value, target)["state"], "pending")
            with patch.object(
                manager.api,
                "request",
                return_value={
                    "state": "pending",
                    "host": target["host"],
                    "id": value["id"],
                    "revision": None,
                    "reason": "Commit unconfirmed",
                },
            ):
                self.assertEqual(manager.attempt(value, target)["state"], "pending")
            with self.assertRaises(ValueError):
                validate_plan(
                    {
                        "action": value["action"],
                        "targets": [{"host": "c" * 64, "ssh": "-oProxyCommand=bad"}],
                    }
                )
            with self.assertRaises(ValueError):
                validate_plan({"action": value["action"], "targets": [target, target]})
            path = Path(directory) / "unsafe.json"
            path.write_text(json.dumps(value))
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                manager.read_private(path)
