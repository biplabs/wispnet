#!/usr/bin/env python3
"""Stable build label shared by all clients; never includes private machine state."""
import hashlib
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def identity():
    version = re.search(r'^VERSION = "([0-9.]+)"', (ROOT / 'apps/node/desktop/wispnet_desktop/version.py').read_text(), re.M)[1]
    revision = subprocess.check_output(['git', 'rev-parse', '--short=12', 'HEAD'], cwd=ROOT, text=True).strip()
    dirty = subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=normal'], cwd=ROOT)
    if not dirty:
        return f'{version}+g{revision}'
    digest = hashlib.sha256()
    paths = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=ROOT).split(b'\0')
    for name in sorted(set(paths) - {b''}):
        path = ROOT / name.decode()
        digest.update(name + b'\0')
        if path.is_file():
            digest.update(str(path.stat().st_mode & 0o111).encode() + b'\0' + path.read_bytes())
        else:
            digest.update(b'<deleted>')
        digest.update(b'\0')
    return f'{version}+g{revision}.dirty.{digest.hexdigest()[:12]}'


if __name__ == '__main__':
    print(identity())
