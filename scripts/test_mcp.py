"""Exercise MCP against a real isolated node and its owner policy API."""
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest

from mcp_headers import headers

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'apps/node/desktop'))
from wispnet_desktop.api import NodeAPI


def unused_port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


class MCPTests(unittest.TestCase):
    def test_generic_grant_preserves_existing_services_and_grants_for_every_kind(self):
        with tempfile.TemporaryDirectory(prefix='wispnet-mcp-grants-') as temporary:
            root = Path(temporary)
            binary = ROOT / 'target/debug/wispnet'

            def host(name):
                config = root / (name + '.toml')
                state = root / name
                config.write_text(f'schema_version=2\ndevice_name="{name}"\nstate_dir="{state}"\nlog_level="info"\n[transport]\nbind_addr="127.0.0.1:0"\n')
                result = subprocess.run([binary, 'host-init', '--config', config],
                                        check=True, capture_output=True, text=True)
                peer = result.stdout.split('Endpoint ID: ')[1].strip()
                return config, state, peer

            config, state, _ = host('owner')
            _, _, existing_peer = host('existing')
            _, _, recipient = host('recipient')
            _, _, unapproved = host('unapproved')
            process = subprocess.Popen([binary, 'agent', '--config', config],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 10
                while not (state / 'node.sock').exists():
                    self.assertIsNone(process.poll())
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(.05)
                api = NodeAPI(config)

                def snapshot():
                    return api.request(op='status')['host']

                def mutate(**operation):
                    return api.request(op='mutate', operation={
                        'expected_revision': snapshot()['revision'], **operation})

                for peer in [existing_peer, recipient]:
                    mutate(op='approve', peer=peer, name='paired-test-device')
                mutate(op='service', id='web', backend='127.0.0.1:18888',
                       allow=[existing_peer], kind='http')
                mutate(op='service', id='raw', backend='127.0.0.1:18889',
                       allow=[existing_peer], kind='tcp')
                mutate(op='shell', allow=[existing_peer])
                port = unused_port()
                api.request(op='configure_access', settings={'registry_port': port})
                api.request(op='enable')
                secret_headers = headers(config)
                request_id = 0

                def rpc(method, params=None):
                    nonlocal request_id
                    request_id += 1
                    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=35)
                    fields = {'Host': f'wispnet.localhost:{port}',
                              'Content-Type': 'application/json',
                              'Accept': 'application/json, text/event-stream', **secret_headers}
                    connection.request('POST', '/mcp', body=json.dumps({
                        'jsonrpc': '2.0', 'id': request_id,
                        'method': method, 'params': params or {}}), headers=fields)
                    reply = connection.getresponse()
                    self.assertEqual(reply.status, 200)
                    result = json.loads(reply.read())
                    connection.close()
                    return result

                def grant(service, peer, revision=None, **extra):
                    return rpc('tools/call', {'name': 'wispnet_grant_service_access',
                        'arguments': {'service': service, 'peer': peer,
                                      'expected_revision': snapshot()['revision'] if revision is None else revision,
                                      **extra}})

                advertised = rpc('tools/list')['result']['tools']
                schema = next(t['inputSchema'] for t in advertised
                              if t['name'] == 'wispnet_grant_service_access')
                self.assertEqual(set(schema['properties']), {'service', 'peer', 'expected_revision'})
                self.assertEqual(set(schema['required']), {'service', 'peer', 'expected_revision'})
                self.assertFalse(schema['additionalProperties'])

                for service in ['web', 'raw', 'shell']:
                    before = snapshot()
                    result = grant(service, recipient, revision=before['revision'])
                    self.assertNotIn('error', result)
                    self.assertFalse(result['result']['isError'], result)
                    self.assertEqual(result['result']['structuredContent']['revision'], before['revision'] + 1)
                    after = snapshot()
                    self.assertEqual(after['clients'], before['clients'])
                    self.assertEqual(after['generation'], before['generation'])
                    for old in before['services']:
                        current = next(s for s in after['services'] if s['id'] == old['id'])
                        if old['id'] == service:
                            self.assertEqual(set(current['allow']), {existing_peer, recipient})
                            self.assertEqual({k: v for k, v in current.items() if k != 'allow'},
                                             {k: v for k, v in old.items() if k != 'allow'})
                        else:
                            self.assertEqual(current, old)
                    stale = grant(service, recipient, revision=before['revision'])
                    self.assertTrue(stale['result']['isError'], stale)
                    self.assertEqual(snapshot(), after)

                # Repeated grants do not duplicate or replace existing grants.
                repeated = grant('shell', recipient)
                self.assertFalse(repeated['result']['isError'], repeated)
                shell = next(s for s in snapshot()['services'] if s['id'] == 'shell')
                self.assertEqual(set(shell['allow']), {existing_peer, recipient})
                self.assertEqual(len(shell['allow']), 2)

                for service, peer in [('missing', recipient), ('web', unapproved)]:
                    before = snapshot()
                    rejected = grant(service, peer)
                    self.assertTrue(rejected['result']['isError'], rejected)
                    self.assertEqual(snapshot(), before)
                before = snapshot()
                invalid = grant('shell', recipient, backend='127.0.0.1:22')
                self.assertEqual(invalid['error']['code'], -32602)
                self.assertEqual(snapshot(), before)
            finally:
                process.terminate()
                process.wait(timeout=10)

    def test_authenticated_owner_changes_revision_conflicts_and_listener_lifecycle(self):
        with tempfile.TemporaryDirectory(prefix='wispnet-mcp-') as temporary:
            root = Path(temporary)
            config = root / 'host.toml'
            state = root / 'state'
            config.write_text(f'schema_version=2\ndevice_name="mcp-test"\nstate_dir="{state}"\nlog_level="info"\n[transport]\nbind_addr="127.0.0.1:0"\n')
            binary = ROOT / 'target/debug/wispnet'
            subprocess.run([binary, 'host-init', '--config', config], check=True, capture_output=True)
            process = subprocess.Popen([binary, 'agent', '--config', config], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 10
                while not (state / 'node.sock').exists():
                    self.assertIsNone(process.poll())
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(.05)
                api = NodeAPI(config)
                port = unused_port()
                api.request(op='configure_access', settings={'registry_port': port})
                api.request(op='enable')
                secret_headers = headers(config)
                original_token = (state / 'mcp-token').read_bytes()
                request_id = 0

                def request(method='POST', path='/mcp', message=None, extra=None, authenticated=True, authority=None):
                    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=35)
                    fields = {'Host': authority or f'wispnet.localhost:{port}', 'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream'}
                    if authenticated:
                        fields.update(secret_headers)
                    fields.update(extra or {})
                    connection.request(method, path, body=json.dumps(message) if message else None, headers=fields)
                    reply = connection.getresponse()
                    status, body = reply.status, reply.read()
                    connection.close()
                    return status, json.loads(body) if body.startswith(b'{') else body

                def rpc(method, params=None):
                    nonlocal request_id
                    request_id += 1
                    status, reply = request(message={'jsonrpc': '2.0', 'id': request_id, 'method': method, 'params': params or {}})
                    self.assertEqual(status, 200)
                    self.assertNotIn('error', reply)
                    return reply['result']

                def call(name, **args):
                    return rpc('tools/call', {'name': name, 'arguments': args})

                initialize = {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-11-25', 'clientInfo': {'name': 'integration', 'version': '1'}, 'capabilities': {}}}
                self.assertEqual(request(message=initialize, authenticated=False)[0], 401)
                self.assertEqual(request(message=initialize, extra={'Origin': 'https://evil.example'})[0], 403)
                self.assertEqual(request(message=initialize)[1]['result']['protocolVersion'], '2025-11-25')
                self.assertIn('wispnet_set_service', [t['name'] for t in rpc('tools/list')['tools']])
                before = call('wispnet_status')['structuredContent']['host']
                operation = dict(expected_revision=before['revision'], id='mcp-fixture', backend='127.0.0.1:18888', allow=[], kind='http')
                created = call('wispnet_set_service', **operation)
                self.assertFalse(created['isError'], created)
                self.assertEqual(created['structuredContent']['revision'], before['revision'] + 1)
                stale = call('wispnet_set_service', **operation)
                self.assertTrue(stale['isError'])
                after = call('wispnet_status')['structuredContent']['host']
                self.assertEqual(after['host'], before['host'])
                self.assertEqual(after['clients'], before['clients'])
                self.assertEqual(next(s['allow'] for s in after['services'] if s['id'] == 'mcp-fixture'), [])
                self.assertEqual(request(method='GET', path='/registry', authenticated=False)[0], 200)
                self.assertEqual(request(method='GET', path='/registry/v1/services', authenticated=False)[0], 200)
                self.assertEqual(request(method='GET', path='/v1/services', authenticated=False, authority=f'registry.wispnet.localhost:{port}')[0], 200)
                self.assertEqual(request(method='GET', path='/registry', authenticated=False, authority='evil.example')[0], 421)
                self.assertFalse(call('wispnet_remove_service', expected_revision=after['revision'], id='mcp-fixture')['isError'])
                changed = unused_port()
                self.assertFalse(call('wispnet_set_port', port=changed)['isError'])
                old_port, port = port, changed
                self.assertEqual(call('wispnet_settings')['structuredContent']['settings']['registry_port'], port)
                with self.assertRaises(OSError):
                    socket.create_connection(('127.0.0.1', old_port), timeout=1)
                self.assertFalse(call('wispnet_set_access', enabled=False)['isError'])
                with self.assertRaises(OSError):
                    socket.create_connection(('127.0.0.1', port), timeout=1)
                api.request(op='enable')
                self.assertEqual((state / 'mcp-token').read_bytes(), original_token)
                self.assertEqual(call('wispnet_status')['structuredContent']['host']['host'], before['host'])
                (state / 'mcp-token').chmod(0o644)
                with self.assertRaises(ValueError):
                    headers(config)
            finally:
                process.terminate()
                process.wait(timeout=10)


if __name__ == '__main__':
    unittest.main()
