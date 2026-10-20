#!/usr/bin/env python3
"""Publish a temporary harmless local fixture, check public routing, then remove it.

Requires an installed owning node authorized by the deployed public gateway.
Use --require-dns to require the generated hostname's own public certificate.
Without it, a missing wildcard record is reported and the relay's verified TLS
hostname is used with the fixture's HTTP Host to test deployed routing.
"""
import argparse
import http.client
import http.server
import json
from pathlib import Path
import socket
import sys
import threading
import time
import urllib.parse
import uuid

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'apps/node/desktop'))
from wispnet_desktop.api import NodeAPI


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', default=str(Path.home() / '.config/wispnet-node/host.toml'))
    parser.add_argument('--require-dns', action='store_true')
    args = parser.parse_args()
    api = NodeAPI(args.config)
    service = 'public-test-' + uuid.uuid4().hex[:6]
    marker = 'wispnet public fixture ' + uuid.uuid4().hex
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            body = marker.encode()
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def do_POST(self):
            body = self.rfile.read(int(self.headers['Content-Length']))
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def log_message(self, *args):
            pass
    backend = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=backend.serve_forever, daemon=True).start()
    created = False
    try:
        snapshot = api.request(op='status')
        api.request(op='mutate', operation={'op': 'service', 'expected_revision': snapshot['host']['revision'],
                    'id': service, 'backend': '127.0.0.1:' + str(backend.server_port), 'allow': [], 'kind': 'http', 'display_name': 'Public test fixture'})
        created = True
        snapshot = api.request(op='status')
        record = next(s for s in snapshot['host']['services'] if s['id'] == service)
        api.request(op='configure_public', service=service, incarnation=record['incarnation'], enabled=True, name=snapshot['host_name'])
        deadline = time.monotonic() + 45
        while True:
            status = api.request(op='public_status')
            publication = next(s for s in status['services'] if s['service'] == service)
            if publication['state'] == 'online':
                break
            if time.monotonic() >= deadline:
                raise RuntimeError('Public registration did not become online: ' + publication['state'])
            time.sleep(.25)
        url = publication['url']
        hostname = urllib.parse.urlsplit(url).hostname
        try:
            socket.getaddrinfo(hostname, 443)
            tls_name = hostname
            print('Public DNS resolves: ' + hostname, flush=True)
        except socket.gaierror:
            if args.require_dns:
                raise RuntimeError('Wildcard DNS is not available for ' + hostname)
            tls_name = 'relay.wispnet.app'
            print('Wildcard DNS pending; testing deployed route through verified relay TLS with the public Host header.', flush=True)
        def request(method='GET', body=None):
            connection = http.client.HTTPSConnection(tls_name, timeout=45)
            connection.request(method, '/', body=body, headers={'Host': hostname, 'Content-Type': 'application/json'})
            response = connection.getresponse()
            result = response.status, response.read().decode()
            connection.close()
            return result
        assert request() == (200, marker)
        assert request('POST', '{"public":"body forwarding"}') == (200, '{"public":"body forwarding"}')
        # The fixture remains private to paired devices; publishing adds no grants.
        owned = next(s for s in api.request(op='status')['host']['services'] if s['id'] == service)
        assert owned['allow'] == []
        api.request(op='configure_public', service=service, enabled=False)
        deadline = time.monotonic() + 5
        while True:
            code, content = request()
            if code != 200 or content != marker:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError('Disabled public route still reached backend')
            time.sleep(.1)
        print('Public GET/POST, unchanged private grants, and disable passed: ' + url, flush=True)
        if tls_name == hostname:
            print('Generated hostname HTTPS certificate verified.', flush=True)
    finally:
        if created:
            api.request(op='configure_public', service=service, enabled=False)
            current = api.request(op='status')
            api.request(op='mutate', operation={'op': 'remove', 'id': service, 'expected_revision': current['host']['revision']})
        backend.shutdown()
        backend.server_close()


if __name__ == '__main__':
    main()
