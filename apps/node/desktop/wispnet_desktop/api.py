"""Bounded owner-local IPC. No shell commands or public management listener."""

import json
import os
from pathlib import Path
import socket
import stat
import tomllib


class NodeRejected(ValueError):
    """Explicit negative acknowledgement from the authenticated local node."""


class NodeAPI:
    def __init__(self, config):
        self.config = Path(config).expanduser().resolve()
        data = tomllib.loads(self.config.read_text())
        self.state = Path(data["state_dir"])
        if not self.state.is_absolute():
            raise ValueError("Node state directory must be absolute")
        self.path = self.state / "node.sock"

    def request(self, **request):
        metadata = self.path.lstat()
        if (
            not stat.S_ISSOCK(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_mode & 0o077
        ):
            raise ValueError("Node socket must be owner-only")
        payload = json.dumps(request, ensure_ascii=True).encode() + b"\n"
        if len(payload) > 16384:
            raise ValueError("Request exceeds local API limit")
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(27)
            connection.connect(str(self.path))
            # Verify the endpoint's OS user as well as the socket's metadata.
            import struct

            _, uid, _ = struct.unpack(
                "3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
            )
            if uid != os.getuid():
                raise ValueError("Unexpected node socket owner")
            connection.sendall(payload)
            data = bytearray()
            while len(data) <= 2 * 1024 * 1024:
                chunk = connection.recv(65536)
                if not chunk:
                    break
                data.extend(chunk)
                if data.endswith(b"\n"):
                    break
            if len(data) > 2 * 1024 * 1024 or not data.endswith(b"\n"):
                raise ValueError("Invalid or oversized node response")
            response = json.loads(data)
            if not response.get("ok"):
                raise NodeRejected(response.get("error", "Node operation failed"))
            result = response["result"]
            if request.get("op") == "status" and result.get("api_version") != 1:
                raise ValueError("Unsupported node API version")
            return result
