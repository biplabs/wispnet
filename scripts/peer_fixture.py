#!/usr/bin/env python3
"""Loopback-only fixture for physical pairwise permission tests."""
import json
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
import sys
import time
class Handler(BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_GET(self):
        self.send_response(200);self.send_header('Connection','close');self.end_headers()
        try:
            if self.path=='/stream':
                while True:self.wfile.write(b'data: alive\n\n');self.wfile.flush();time.sleep(.2)
            else:self.wfile.write(('Peer fixture: '+sys.argv[1]).encode())
        except (BrokenPipeError,ConnectionResetError):pass
server=ThreadingHTTPServer(('127.0.0.1',0),Handler);server.daemon_threads=True
print(json.dumps({'port':server.server_port}),flush=True)
server.serve_forever()
