#!/usr/bin/env python3
"""Run the real node integration contract through the Tauri app's IPC client."""

import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

import test_node_desktop as integration

ROOT = Path(__file__).resolve().parents[1]
NATIVE = Path(
    os.environ.get(
        "WISPNET_TAURI_BINARY", ROOT / "apps/macos/src-tauri/target/release/Wispnet"
    )
)
if os.environ.get("WISPNET_NODE_BINARY"):
    integration.BINARY = Path(os.environ["WISPNET_NODE_BINARY"])
PythonAPI = integration.module.NodeAPI


class NativeAPI(PythonAPI):
    def request(self, **request):
        result = subprocess.run(
            [str(NATIVE), "--request", str(self.path)],
            input=json.dumps(request),
            text=True,
            capture_output=True,
            timeout=35,
        )
        if result.returncode:
            if not self.path.exists():
                raise OSError(result.stderr.strip())
            raise ValueError(result.stderr.strip())
        return json.loads(result.stdout)


if __name__ == "__main__":
    integration.module.NodeAPI = NativeAPI
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(
        integration.NodeIntegration
    )
    raise SystemExit(
        not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful()
    )
