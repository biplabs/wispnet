#!/usr/bin/env python3
"""Disposable loopback HTTP fixture for opt-in installed-host browser acceptance."""

import argparse
import base64
import hashlib
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import struct
import time
import threading


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def reply(self, status, body=b"", headers=()):
        self.send_response(status)
        for key, value in headers:
            self.send_header(key, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.startswith("/attack") and self.path != "/attacks":
            with self.server.counter_lock:
                self.server.attacks += 1
        if self.path == "/attacks":
            self.reply(200, str(self.server.attacks).encode())
        elif self.path == "/writes":
            self.reply(200, str(self.server.writes).encode())
        elif self.path == "/inspect":
            self.reply(
                200,
                json.dumps(dict(self.headers)).encode(),
                [("Content-Type", "application/json")],
            )
        elif self.path == "/cookie":
            self.reply(
                200,
                b"ok",
                [
                    (
                        "Set-Cookie",
                        "app=" + self.server.label + "; Domain=.localhost; Path=/",
                    ),
                    ("Set-Cookie", "_wispnet_session=forged; Path=/"),
                ],
            )
        elif self.path == "/login":
            self.reply(
                302,
                headers=[
                    ("Location", "/"),
                    ("Set-Cookie", "login=yes; Path=/; HttpOnly; SameSite=Strict"),
                ],
            )
        elif self.path == "/secure":
            self.reply(
                200, headers=[("Set-Cookie", "__Host-test=secret; Secure; Path=/")]
            )
        elif self.path == "/external":
            self.reply(302, headers=[("Location", "https://example.com/")])
        elif self.path == "/sw.js":
            self.reply(
                200,
                b'self.addEventListener("fetch",()=>{});',
                [("Content-Type", "text/javascript")],
            )
        elif self.path == "/events":
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            self.wfile.write(b"d\r\ndata: hello\n\n\r\n0\r\n\r\n")
            self.wfile.flush()
        elif self.path == "/ws":
            key = self.headers["Sec-WebSocket-Key"]
            accept = base64.b64encode(
                hashlib.sha1(
                    (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()
                ).digest()
            ).decode()
            self.send_response(101)
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", accept)
            self.end_headers()
            first, second = self.rfile.read(2)
            size = second & 127
            if size == 126:
                size = struct.unpack("!H", self.rfile.read(2))[0]
            assert first == 0x81 and second & 128 and size < 1024
            mask = self.rfile.read(4)
            data = self.rfile.read(size)
            data = bytes(v ^ mask[i % 4] for i, v in enumerate(data))
            self.wfile.write(bytes([0x81, len(data)]) + data)
            self.wfile.flush()
            self.close_connection = True
        else:
            self.reply(
                200,
                (
                    "<!doctype html><title>gateway fixture</title><body>"
                    + self.server.label
                ).encode(),
                [("Content-Type", "text/html")],
            )

    def do_POST(self):
        if self.path == "/attack":
            with self.server.counter_lock:
                self.server.attacks += 1
            self.reply(200)
            return
        assert self.path == "/write"
        self.rfile.read(int(self.headers.get("Content-Length", "0")))
        with self.server.counter_lock:
            self.server.writes += 1
        time.sleep(3)
        try:
            self.reply(200, b"committed")
        except (BrokenPipeError, ConnectionResetError):
            pass


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--ready", required=True)
    parser.add_argument("--label", required=True)
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.label = args.label
    server.writes = 0
    server.attacks = 0
    server.counter_lock = threading.Lock()
    Path(args.ready).write_text(str(server.server_port))
    server.serve_forever()
