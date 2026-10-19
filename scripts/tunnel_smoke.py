#!/usr/bin/env python3
"""Two real Linux CLI processes over direct Iroh; local deterministic HTTP backend."""
import json
import os
from pathlib import Path
import select
import signal
import socket
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

binary = Path("target/debug/wispnet").resolve()
accepted = threading.Event()
post_count = 0
class Backend(ThreadingHTTPServer):
    daemon_threads = True
    def get_request(self):
        result = super().get_request()
        accepted.set()
        return result

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass
    def do_GET(self):
        self.send_response(200)
        self.send_header("Connection", "close")
        self.send_header("Content-Type", "text/event-stream" if self.path == "/events" else "text/html")
        self.end_headers()
        if self.path == "/events":
            self.wfile.write(b"data: first\n\n")
            self.wfile.flush()
            release.wait(5)
            self.wfile.write(b"data: second\n\n")
        else:
            self.wfile.write(b"<h1>real process page</h1>")
    def do_POST(self):
        global post_count
        self.rfile.read(int(self.headers.get("Content-Length", "0")))
        post_count += 1
        # Simulate failure after a side effect, before an HTTP response.
        self.close_connection = True

server = Backend(("127.0.0.1", 0), Handler)
release = threading.Event()
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
processes = []
try:
    with tempfile.TemporaryDirectory(prefix="wispnet-tunnel-") as temporary:
        root = Path(temporary)
        def config(name):
            path = root / f"{name}.toml"
            path.write_text(f'schema_version=1\ndevice_name="{name}"\nstate_dir="{root / name}"\nlog_level="debug"\n')
            result = subprocess.run([binary, "identity-init", "--config", path], capture_output=True, text=True, timeout=5)
            assert result.returncode == 0, result.stderr
            return path, result.stdout.split("Endpoint ID: ")[1].strip()
        host_config, host_id = config("host")
        client_config, client_id = config("client")
        unknown_config, _ = config("unknown")
        host_config.write_text(host_config.read_text() +
            f'\n[transport]\nbind_addr="127.0.0.1:0"\n'
            f'\n[[services]]\nid="website"\nbackend="127.0.0.1:{server.server_port}"\nallow=["{client_id}"]\n')
        host = subprocess.Popen([binary, "agent", "--config", host_config], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        processes.append(host)
        assert select.select([host.stderr], [], [], 10)[0], "agent startup timed out"
        started = json.loads(host.stderr.readline())
        assert started["fields"]["event"] == "started" and started["fields"]["networking"]
        address = started["fields"]["bind_addr"]
        def command(path=client_config, service="website"):
            return [binary, "tunnel", "--config", path, "--peer", host_id, "--address", address, "--service", service]
        result = subprocess.run(command(), input=b"GET / HTTP/1.1\r\nHost: website\r\n\r\n", capture_output=True, timeout=10)
        assert result.returncode == 0, result.stderr
        assert b"<h1>real process page</h1>" in result.stdout
        for path, service in [(unknown_config, "website"), (client_config, "missing")]:
            result = subprocess.run(command(path, service), input=b"GET / HTTP/1.1\r\n\r\n", capture_output=True, timeout=10)
            assert result.returncode != 0
        stream = subprocess.Popen(command(), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        processes.append(stream)
        stream.stdin.write(b"GET /events HTTP/1.1\r\nHost: website\r\n\r\n")
        stream.stdin.close()
        output = bytearray()
        # Read before releasing the backend: this cannot pass if the tunnel buffers
        # the entire response.
        while b"data: first\n\n" not in output:
            assert select.select([stream.stdout], [], [], 5)[0], "first SSE event was buffered"
            byte = os.read(stream.stdout.fileno(), 16384)
            assert byte, "stream ended early"
            output += byte
        release.set()
        output += stream.stdout.read()
        assert b"data: second\n\n" in output
        stream.wait(timeout=5)
        assert stream.returncode == 0, stream.stderr.read()

        request = b"POST /effect HTTP/1.1\r\nHost: website\r\nContent-Length: 16\r\n\r\nPAYLOAD-SENTINEL"
        result = subprocess.run(command(), input=request, capture_output=True, timeout=10)
        assert post_count == 1, "request was replayed after backend EOF"
        assert b"PAYLOAD-SENTINEL" not in result.stderr

        accepted.clear()
        blocked = subprocess.Popen(command(), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        processes.append(blocked)
        assert accepted.wait(5), "client never opened authorized backend"
        # Leave stdin open and unread. Signal shutdown must not wait for a blocking
        # stdin worker to finish.
        blocked.send_signal(signal.SIGTERM)
        blocked.wait(timeout=5)
        assert blocked.returncode != 0
        blocked.stdin.close()
        assert b"tunnel cancelled" in blocked.stderr.read()

        host.send_signal(signal.SIGTERM)
        _, errors = host.communicate(timeout=5)
        assert host.returncode == 0, errors
        assert '"stopped"' in errors
        ip, port = address.rsplit(":", 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.bind((ip, int(port)))  # verify the Iroh listener is gone
        assert post_count == 1
finally:
    release.set()
    for process in processes:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
    server.shutdown()
    server.server_close()
    worker.join(timeout=5)
print("Two-process Iroh tunnel smoke checks passed")
