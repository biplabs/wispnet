"""Bounded private display snapshots. Pins and permission authority live elsewhere."""

import json
import math
import os
from pathlib import Path
import re
import stat
import tempfile

MAX_BYTES = 128 * 1024


def valid_catalog(peer, value):
    return (
        isinstance(value, dict)
        and set(value)
        == {
            "version",
            "complete",
            "service_count",
            "host",
            "generation",
            "revision",
            "services",
        }
        and type(value["version"]) is int
        and value["version"] == 1
        and value["complete"] is True
        and type(value["service_count"]) is int
        and value["host"] == peer
        and isinstance(value["generation"], str)
        and re.fullmatch(r"[0-9a-f]{64}", value["generation"])
        and type(value["revision"]) is int
        and 0 <= value["revision"] < 2**63
        and isinstance(value["services"], list)
        and len(value["services"]) <= 130
        and value["service_count"] == len(value["services"])
        and all(
            isinstance(s, dict)
            and set(s) == {"id", "incarnation", "display_name", "kind"}
            and s["kind"] in ("http", "tcp", "shell", "files", "file_share")
            and isinstance(s["display_name"], str)
            and bool(s["display_name"].strip())
            and len(s["display_name"].encode("utf-8", errors="surrogatepass")) <= 128
            and not any(
                ord(c) < 32 or 127 <= ord(c) <= 159 or 0xD800 <= ord(c) <= 0xDFFF
                for c in s["display_name"]
            )
            and isinstance(s["incarnation"], str)
            and re.fullmatch(r"[0-9a-f]{64}", s["incarnation"])
            and isinstance(s["id"], str)
            and re.fullmatch(r"[a-z0-9][a-z0-9._-]{0,63}", s["id"])
            for s in value["services"]
        )
        and [s["id"] for s in value["services"]]
        == sorted({s["id"] for s in value["services"]})
    )


class CatalogCache:
    def __init__(self, state):
        self.directory = Path(state) / "desktop-catalogs" if state else None

    def path(self, peer):
        if not re.fullmatch(r"[0-9a-f]{64}", peer):
            raise ValueError("Invalid peer ID")
        return self.directory / (peer + ".json") if self.directory else None

    def load(self, peer):
        path = self.path(peer)
        if path is None or not path.exists():
            return None
        info = self.directory.lstat()
        if (
            not stat.S_ISDIR(info.st_mode)
            or info.st_uid != os.getuid()
            or info.st_mode & 0o077
        ):
            raise ValueError("Unsafe catalog cache directory")
        # Reject symlinks, foreign ownership, public permissions and oversized data.
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, "rb") as source:
            info = os.fstat(source.fileno())
            if (
                not stat.S_ISREG(info.st_mode)
                or info.st_uid != os.getuid()
                or info.st_mode & 0o077
                or info.st_size > MAX_BYTES
            ):
                raise ValueError("Unsafe catalog cache")
            value = json.loads(source.read(MAX_BYTES + 1))
        if (
            not isinstance(value, dict)
            or set(value) != {"version", "catalog", "fetched"}
            or type(value["version"]) is not int
            or value["version"] != 1
            or not valid_catalog(peer, value["catalog"])
            or type(value["fetched"]) not in (int, float)
            or not math.isfinite(value["fetched"])
            or value["fetched"] < 0
        ):
            raise ValueError("Invalid catalog cache")
        return {
            "catalog": value["catalog"],
            "fetched": value["fetched"],
            "state": "Cached; refresh required",
        }

    def save(self, peer, entry):
        path = self.path(peer)
        if path is None:
            return
        if not valid_catalog(peer, entry["catalog"]):
            raise ValueError("Invalid catalog")
        data = json.dumps(
            {"version": 1, "catalog": entry["catalog"], "fetched": entry["fetched"]}
        ).encode()
        if len(data) > MAX_BYTES:
            raise ValueError("Catalog too large")
        self.directory.mkdir(mode=0o700, exist_ok=True)
        info = self.directory.lstat()
        if (
            not stat.S_ISDIR(info.st_mode)
            or info.st_uid != os.getuid()
            or info.st_mode & 0o077
        ):
            raise ValueError("Unsafe catalog cache directory")
        fd, name = tempfile.mkstemp(prefix="snapshot-", dir=self.directory)
        try:
            with os.fdopen(fd, "wb") as output:
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
            os.replace(name, path)
            directory_fd = os.open(
                self.directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
            )
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
        finally:
            if os.path.exists(name):
                os.unlink(name)

    def remove(self, peer):
        path = self.path(peer)
        if path is not None:
            path.unlink(missing_ok=True)
