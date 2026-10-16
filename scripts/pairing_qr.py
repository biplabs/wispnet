#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["qrcode==8.2"]
# ///
"""Render a protected host invitation as a private QR PNG; never print secrets."""
import argparse
import base64
import json
import os
from pathlib import Path
import stat
import struct
import time
import zlib


def render(source, destination):
    import qrcode
    source, destination = Path(source), Path(destination)
    if not source.is_absolute() or not destination.is_absolute():
        raise ValueError('Use absolute paths')
    fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_nlink != 1 or metadata.st_mode & 0o077 or metadata.st_size > 8192:
            raise ValueError('Input must be a protected invitation file')
        text = stream.read(8193).decode('utf-8').strip()
    if len(text.encode()) > 8192:
        raise ValueError('Invitation too large')
    if text.startswith('wispnet1:'):
        code = text[len("wispnet1:"):]
        raw = base64.b64decode(code + '=' * (-len(code) % 4), altchars=b'-_', validate=True)
    else:
        raw = text.encode()
    invitation = json.loads(raw)
    if invitation.get('version') != 1 or invitation.get('scope') != 'request_access' or invitation.get('expires_at', 0) <= time.time():
        raise ValueError('Invalid or expired invitation')
    payload = 'wispnet1:' + base64.urlsafe_b64encode(raw).decode().rstrip('=')
    qr = qrcode.QRCode(error_correction=qrcode.constants.ERROR_CORRECT_M, box_size=8, border=4)
    qr.add_data(payload); qr.make(fit=True)
    matrix = qr.get_matrix(); width = len(matrix) * 8
    # Grayscale PNG, using the standard library instead of another image runtime.
    rows = b''.join((b'\0' + bytes(0 if dark else 255 for dark in row for _ in range(8))) * 8 for row in matrix)
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, width, 8, 0, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')
    with os.fdopen(os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb') as output:
        output.write(png); output.flush(); os.fsync(output.fileno())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', required=True); parser.add_argument('--output', required=True)
    args = parser.parse_args()
    try:
        render(args.input, args.output)
    except Exception:
        parser.exit(1, 'Cannot render QR: use a valid private, unexpired invitation and a new output path.\n')
    print('Private QR image created. Compare the target host ID before pairing.')


if __name__ == '__main__':
    main()
