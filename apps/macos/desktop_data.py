#!/usr/bin/env python3
"""Private desktop persistence and existing owner operations, without Qt.

Only fixed operations are exposed; plans travel on stdin, never through a shell.
"""

import configparser
import json
import os
import re
from pathlib import Path
import sys
import time

installed = Path(__file__).resolve().parent
source = installed.parents[1] / "apps/node/desktop"
sys.path.insert(0, str(source if source.exists() else installed))
from wispnet_desktop.api import NodeAPI
from wispnet_desktop.catalog_cache import CatalogCache
from wispnet_desktop.combined import Combined, validate_plan


def dispatch(payload, config=None):
    config = config or Path.home() / ".config/wispnet-node/host.toml"
    manager = Combined(config)
    cache = CatalogCache(manager.api.state)
    op = payload["op"]
    if op == "operations":
        return manager.list()
    if op == "targets":
        path = Path(config).parent / "admin-targets.json"
        targets = (
            manager.read_private(path)
            if path.exists()
            else [{"host": manager.api.request(op="status")["host"]["host"]}]
        )
        validate_plan(
            {"action": {"op": "revoke", "peer": "0" * 64}, "targets": targets}
        )
        return targets
    if op == "start":
        return manager.start(payload["plan"])
    if op == "retry":
        return manager.retry(payload["id"])
    if op == "cache_load":
        return cache.load(payload["host"])
    if op == "cache_save":
        cache.save(
            payload["host"], {"catalog": payload["catalog"], "fetched": time.time()}
        )
        return None
    if op == "cache_remove":
        cache.remove(payload["host"])
        return None
    if op == "preferences":
        names = {}
        try:
            names = manager.read_private(manager.api.state / "desktop-peer-names.json")
        except FileNotFoundError:
            pass
        names = (
            {
                peer: label
                for peer, label in names.items()
                if re.fullmatch(r"[0-9a-f]{64}", peer)
                and isinstance(label, str)
                and 0 < len(label) <= 128
                and not any(ord(c) < 32 for c in label)
            }
            if isinstance(names, dict)
            else {}
        )
        order = {}
        path = (
            Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
            / "wispnet/desktop-order.conf"
        )
        if path.exists():
            parser = configparser.ConfigParser(interpolation=None)
            parser.optionxform = str
            parser.read(path)
            for section in parser.sections():
                for key, value in parser.items(section):
                    group = key if section == "General" else section + "/" + key
                    order[group] = [v.strip() for v in value.split(",") if v.strip()]
        return {"names": names, "order": order}
    raise ValueError("Unsupported desktop operation")


if __name__ == "__main__":
    try:
        data = sys.stdin.buffer.read(65537)
        if len(data) > 65536:
            raise ValueError("Desktop request too large")
        print(json.dumps(dispatch(json.loads(data))))
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
