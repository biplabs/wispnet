#!/usr/bin/env python3
"""Send a JSON request to the running owner-local node; no Qt or second endpoint."""

import argparse
import json
from pathlib import Path
import sys

from wispnet_desktop.api import NodeAPI


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--config", type=Path, default=Path.home() / ".config/wipsnet-node/host.toml"
    )
    args = parser.parse_args()
    try:
        text = (
            "" if sys.stdin.isatty() else sys.stdin.buffer.read(16385).decode("utf-8")
        )
        if len(text.encode("utf-8")) > 16384:
            raise ValueError("Request exceeds local API limit")
        request = json.loads(text) if text.strip() else {"op": "status"}
        if not isinstance(request, dict):
            raise ValueError("Request must be a JSON object")
        result = NodeAPI(args.config).request(**request)
        print(json.dumps(result, indent=2))
    except (OSError, ValueError, TypeError) as error:
        print("Node request failed: " + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
