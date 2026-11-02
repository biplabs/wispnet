import json
import os
from pathlib import Path
import socket
import stat
import struct
import unittest
from unittest.mock import Mock, patch

from wispnet_desktop.api import NodeAPI


@unittest.skipUnless(hasattr(socket, "SO_PEERCRED"), "Linux peer credentials")
class RequestBudgetChecks(unittest.TestCase):
    def test_only_the_native_connection_probe_has_an_extended_socket_budget(self):
        api = NodeAPI.__new__(NodeAPI)
        api.path = Path("/private/node.sock")
        metadata = Mock(st_mode=stat.S_IFSOCK | 0o600, st_uid=os.getuid())
        for operation, expected in (("diagnose_connection", 65), ("status", 27), ("mutate", 27)):
            with self.subTest(operation=operation):
                connection = Mock()
                connection.getsockopt.return_value = struct.pack("3i", 1, os.getuid(), os.getgid())
                connection.recv.return_value = json.dumps({"ok": True, "result": {"api_version": 1}}).encode() + b"\n"
                context = Mock()
                context.__enter__ = Mock(return_value=connection)
                context.__exit__ = Mock(return_value=False)
                with patch.object(Path, "lstat", return_value=metadata), \
                     patch("wispnet_desktop.api.socket.socket", return_value=context):
                    self.assertEqual(api.request(op=operation, timeout=600), {"api_version": 1})
                connection.settimeout.assert_called_once_with(expected)

    def test_diagnostic_budget_preserves_owner_credential_validation(self):
        api = NodeAPI.__new__(NodeAPI)
        api.path = Path("/private/node.sock")
        metadata = Mock(st_mode=stat.S_IFSOCK | 0o600, st_uid=os.getuid())
        connection = Mock()
        connection.getsockopt.return_value = struct.pack("3i", 1, os.getuid() + 1, os.getgid())
        context = Mock()
        context.__enter__ = Mock(return_value=connection)
        context.__exit__ = Mock(return_value=False)
        with patch.object(Path, "lstat", return_value=metadata), \
             patch("wispnet_desktop.api.socket.socket", return_value=context):
            with self.assertRaisesRegex(ValueError, "Unexpected node socket owner"):
                api.request(op="diagnose_connection", host="a" * 64)
        connection.sendall.assert_not_called()
