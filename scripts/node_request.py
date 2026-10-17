#!/usr/bin/env python3
"""Send a JSON request to the running owner-local node; no Qt or second endpoint."""

import argparse
import json
from pathlib import Path
import sys

from wispnet_desktop.api import NodeAPI, NodeRejected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--config", type=Path, default=Path.home() / ".config/wipsnet-node/host.toml"
    )
    parser.add_argument(
        "--response",
        action="store_true",
        help="Emit an explicit acknowledgement envelope for owner automation",
    )
    operations = parser.add_mutually_exclusive_group()
    operations.add_argument(
        "--batch", type=Path, help="Create/run a private combined-operation plan"
    )
    operations.add_argument(
        "--retry", help="Retry pending targets of a saved operation"
    )
    operations.add_argument(
        "--operations",
        action="store_true",
        help="List durable combined-operation status",
    )
    args = parser.parse_args()
    try:
        if args.batch or args.retry or args.operations:
            from wispnet_desktop.combined import Combined

            manager = Combined(args.config)
            if args.batch:
                result = manager.start(manager.read_private(args.batch))
            elif args.retry:
                result = manager.retry(args.retry)
            else:
                result = manager.list()
            print(json.dumps(result, indent=2))
            return 0
        text = (
            "" if sys.stdin.isatty() else sys.stdin.buffer.read(16385).decode("utf-8")
        )
        if len(text.encode("utf-8")) > 16384:
            raise ValueError("Request exceeds local API limit")
        request = json.loads(text) if text.strip() else {"op": "status"}
        if not isinstance(request, dict):
            raise ValueError("Request must be a JSON object")
        result = NodeAPI(args.config).request(**request)
        print(
            json.dumps(
                {"ok": True, "result": result} if args.response else result, indent=2
            )
        )
    except (OSError, ValueError, TypeError, KeyError) as error:
        if args.response:
            print(
                json.dumps(
                    {
                        "ok": False,
                        "kind": "rejected"
                        if isinstance(error, NodeRejected)
                        else "unavailable",
                        "error": str(error),
                    }
                )
            )
        else:
            print("Node request failed: " + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
