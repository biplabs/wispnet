#!/usr/bin/env python3
"""Real CLI Files checks over two isolated native Iroh/QUIC nodes.

Temporary identities, policies, homes, exports and loopback pairing hints keep
this test independent of installed nodes and the user's existing grants.
"""

import hashlib
import json
import os
import subprocess
import unittest

import shell_smoke


class FilesSmoke(unittest.TestCase):
    # Reuse the isolated node fixture without inheriting its Shell test cases.
    setUp = shell_smoke.ShellSmoke.setUp
    tearDown = shell_smoke.ShellSmoke.tearDown
    host = shell_smoke.ShellSmoke.host
    cli = shell_smoke.ShellSmoke.cli
    mutate = shell_smoke.ShellSmoke.mutate

    def completed(self, result, expected_hash, size):
        completion = json.loads(result.stdout)
        self.assertEqual(completion["state"], "complete", completion)
        self.assertEqual(completion["file"]["sha256"], expected_hash)
        self.assertEqual(completion["file"]["size_bytes"], size)
        self.assertTrue(completion["file"]["file_version"])
        progress = [json.loads(line) for line in result.stderr.splitlines()]
        self.assertTrue(progress, "Files CLI emitted no machine-readable progress")
        self.assertEqual(progress[-1]["state"], "complete")
        self.assertEqual(progress[-1]["verified_bytes"], size)
        # Progress and completion contain metadata, never the file bytes.
        self.assertLess(len(result.stdout), 8192)
        self.assertTrue(all(len(line) < 8192 for line in result.stderr.splitlines()))
        return completion

    def test_native_cli_transfer_grants_retries_and_path_confinement(self):
        client, api, client_id, _ = self.host("files-client")
        target, target_api, target_id, target_port = self.host("files-target")
        export = self.root.resolve() / "export"
        export.mkdir(mode=0o700)
        source = self.root / "dataset.bin"
        content = os.urandom(5 * 1024 * 1024 + 137)
        source.write_bytes(content)
        expected_hash = hashlib.sha256(content).hexdigest()

        initial = json.loads(self.cli(target, "files-access").stdout)
        self.assertEqual(initial["service"]["id"], "files")
        self.assertEqual(initial["service"]["kind"], "files")
        self.assertEqual(initial["service"]["allow"], [])
        self.assertEqual(initial["exports"], {})
        invitation_path = self.root / "invitation.json"
        subprocess.run(
            [shell_smoke.BINARY, "host-invite", "--config", target,
             "--output", invitation_path],
            capture_output=True, check=True, env=self.environment, timeout=15,
        )
        invitation = json.loads(invitation_path.read_text())
        invitation["direct_hint"] = f"127.0.0.1:{target_port}"
        api.request(op="pair", host=target_id, invitation=json.dumps(invitation))
        self.mutate(target_api, op="approve_invitation",
                    invitation=invitation["invitation"])

        # Pairing and a Shell grant must not grant Files access.
        self.cli(target, "shell-access", "--allow", client_id)
        catalog = api.request(op="catalog", host=target_id)
        self.assertEqual([service["id"] for service in catalog["services"]], ["shell"])
        # A folder must be configured before any UI/CLI/API can grant Files.
        before_folder = target_api.request(op="status")["host"]
        no_folder = self.cli(target, "files-access", "--allow", client_id,
                             success=False)
        self.assertIn("folder", no_folder.stderr.lower())
        self.assertEqual(target_api.request(op="status")["host"], before_folder)
        with self.assertRaises(ValueError):
            self.mutate(target_api, op="grant_service", service="files",
                        peer=client_id)
        self.assertEqual(target_api.request(op="status")["host"], before_folder)
        self.assertEqual(api.request(op="catalog", host=target_id)["services"],
                         catalog["services"])

        denied = self.cli(client, "files", "put", target_id, "training.v1",
                          str(source), "jobs/a/input.bin", success=False)
        self.assertIn("not granted", denied.stderr)
        self.assertEqual(json.loads(denied.stdout)["error"]["code"], "PERMISSION_DENIED")
        self.assertFalse((export / "jobs/a/input.bin").exists())

        configured = json.loads(self.cli(
            target, "files-access", "--export", f"training.v1={export}",
            "--allow", "none",
        ).stdout)
        self.assertEqual(configured["exports"], {"training.v1": str(export)})
        granted = json.loads(self.cli(target, "files-access", "--allow", client_id).stdout)
        self.assertEqual(granted["allow"], [client_id])
        self.assertEqual(granted["exports"], configured["exports"])
        catalog = api.request(op="catalog", host=target_id)
        files_service = next(service for service in catalog["services"]
                             if service["id"] == "files")
        workspaces = api.request(op="file_workspaces", host=target_id,
                                 incarnation=files_service["incarnation"])
        self.assertEqual(workspaces, {"workspaces": ["training.v1"]})
        self.assertNotIn(str(export), json.dumps(workspaces))
        put = self.cli(client, "files", "put", target_id, "training.v1",
                       str(source), "jobs/a/input.bin")
        uploaded = self.completed(put, expected_hash, len(content))
        self.assertEqual(hashlib.sha256((export / "jobs/a/input.bin").read_bytes()).hexdigest(), expected_hash)
        self.assertEqual(uploaded["file"]["path"], "jobs/a/input.bin")

        # A grant is ongoing: retries and further sends need no new confirmation.
        # Repeating a completed helper call recognizes the same destination.
        retry = self.cli(client, "files", "put", target_id, "training.v1",
                         str(source), "jobs/a/input.bin")
        self.completed(retry, expected_hash, len(content))
        additional = self.cli(client, "files", "put", target_id, "training.v1",
                              str(source), "jobs/b/input.bin")
        self.completed(additional, expected_hash, len(content))
        self.assertEqual((export / "jobs/b/input.bin").read_bytes(), content)

        # Receiving permission never lets the sender retrieve the host's files.
        downloaded = self.root / "downloaded.bin"
        get = self.cli(client, "files", "get", target_id, "training.v1",
                       str(downloaded), "jobs/a/input.bin", success=False)
        self.assertFalse(downloaded.exists())
        self.assertIn("send", get.stderr.lower())

        # Identical existing content is not this sender's completed transfer.
        existing = export / "existing.bin"
        existing.write_bytes(content)
        conflict = self.cli(client, "files", "put", target_id, "training.v1",
                            str(source), "existing.bin", success=False)
        self.assertEqual(json.loads(conflict.stdout)["error"]["code"], "CONFLICT")
        self.assertEqual(existing.read_bytes(), content)

        outside = self.root / "outside.bin"
        escaped = self.cli(client, "files", "put", target_id, "training.v1",
                           str(source), "../outside.bin", success=False)
        failure = json.loads(escaped.stdout)
        self.assertEqual(failure["state"], "failed")
        self.assertEqual(failure["error"]["code"], "PATH_DENIED")
        self.assertFalse(outside.exists())

        # Files revocation leaves Shell access and the completed artifact intact.
        self.cli(target, "files-access", "--allow", "none")
        catalog = api.request(op="catalog", host=target_id)
        self.assertEqual([service["id"] for service in catalog["services"]], ["shell"])
        with self.assertRaisesRegex(ValueError, "PERMISSION_DENIED"):
            api.request(op="file_workspaces", host=target_id,
                        incarnation=files_service["incarnation"])
        revoked = self.cli(client, "files", "put", target_id, "training.v1",
                           str(source), "jobs/revoked.bin", success=False)
        self.assertIn("not granted", revoked.stderr)
        self.assertEqual(json.loads(revoked.stdout)["error"]["code"], "PERMISSION_DENIED")
        self.assertFalse((export / "jobs/revoked.bin").exists())
        self.assertEqual(hashlib.sha256((export / "jobs/a/input.bin").read_bytes()).hexdigest(), expected_hash)

        # Removing the last folder revokes standing access without deleting data.
        self.cli(target, "files-access", "--allow", client_id)
        before_clear = target_api.request(op="status")["host"]
        conflicting_clear = self.cli(target, "files-access", "--clear-exports",
                                     "--allow", client_id, success=False)
        self.assertIn("folder", conflicting_clear.stderr.lower())
        self.assertEqual(target_api.request(op="status")["host"], before_clear)
        cleared = json.loads(self.cli(target, "files-access", "--clear-exports").stdout)
        self.assertEqual(cleared["allow"], [])
        self.assertEqual(cleared["exports"], {})
        snapshot = target_api.request(op="status")["host"]
        self.assertEqual(snapshot["file_exports"], {})
        files = next(service for service in snapshot["services"]
                     if service["id"] == "files")
        self.assertEqual(files["allow"], [])
        catalog = api.request(op="catalog", host=target_id)
        self.assertEqual([service["id"] for service in catalog["services"]], ["shell"])
        self.assertEqual(hashlib.sha256((export / "jobs/a/input.bin").read_bytes()).hexdigest(), expected_hash)


if __name__ == "__main__":
    unittest.main()
