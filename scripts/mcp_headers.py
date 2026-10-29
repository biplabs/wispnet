#!/usr/bin/env python3
"""Compatibility helper for older MCP configs; local MCP needs no credentials."""
import argparse
import json
from pathlib import Path


def headers(config=None):
    # Keep existing http_headers_helper configurations working without reading
    # or sending the obsolete host-local MCP credential.
    return {}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, default=Path.home() / '.config/wispnet-node/host.toml')
    args = parser.parse_args()
    print(json.dumps(headers(args.config)))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
