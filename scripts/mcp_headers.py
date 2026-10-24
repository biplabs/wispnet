#!/usr/bin/env python3
"""Codex HTTP-header helper. Its stdout is a credential; pipe only to an MCP client."""
import argparse
import json
import os
from pathlib import Path
import re
import stat
import sys
import tomllib


def headers(config):
    settings = tomllib.loads(Path(config).read_text())
    state = Path(settings['state_dir'])
    if not state.is_absolute():
        raise ValueError('Node state directory must be absolute')
    metadata = state.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        raise ValueError('Node state directory must be owner-only')
    fd = os.open(state / 'mcp-token', os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'r') as source:
        metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_nlink != 1 or metadata.st_size != 64:
            raise ValueError('MCP credential must be an owner-only regular file')
        token = source.read(65)
    if not re.fullmatch('[0-9a-f]{64}', token):
        raise ValueError('Invalid MCP credential')
    return {'Authorization': 'Bearer ' + token}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, default=Path.home() / '.config/wispnet-node/host.toml')
    args = parser.parse_args()
    try:
        print(json.dumps(headers(args.config)))
    except (OSError, ValueError, KeyError):
        print('Wispnet MCP credential unavailable; start local access and check the node configuration.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
