import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('flutter_bridge', Path(__file__).with_name('bridge.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
OWN, PEER, INCARNATION = 'a' * 64, 'b' * 64, 'c' * 64


def catalog():
    return dict(version=1, complete=True, service_count=1, host=PEER, generation='d' * 64, revision=3,
                services=[dict(id='web', incarnation=INCARNATION, display_name='Website', kind='http')])


class BridgeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.state = self.root / 'state'
        self.state.mkdir(mode=0o700)
        self.config = self.root / 'host.toml'
        self.config.write_text('state_dir=' + json.dumps(str(self.state)))
        self.env = patch.dict(os.environ, XDG_DATA_HOME=str(self.root / 'data'), XDG_CONFIG_HOME=str(self.root / 'config'))
        self.env.start()
        self.addCleanup(self.env.stop)
        self.status = dict(host=dict(host=OWN, revision=7, services=[], clients=[dict(peer=PEER, name='Saved host')]),
                           host_name='Desktop', peer_pins=[PEER], invitations=[], relay='connected', peer_telemetry={PEER: dict(path='direct', rtt_ms=15)})
        self.gateway = dict(settings=dict(registry_port=18355, service_ports=[dict(host=PEER, service='other', port=18356)]), routes=[])
        self.calls = []
        def request(**args):
            self.calls.append(args)
            if args['op'] == 'status': return self.status
            if args['op'] == 'gateway_settings': return self.gateway
            if args['op'] == 'catalog': return catalog()
            if args['op'] == 'open': return dict(url='http://web.localhost:18000/')
            return {}
        self.mock = patch.object(module.NodeAPI, 'request', side_effect=request).start()
        self.addCleanup(patch.stopall)
        self.bridge = module.Bridge(self.config)

    def test_upgrade_preserves_private_order_names_and_cache_without_trusting_cache(self):
        self.bridge.dispatch('saveOrder', dict(group='machines', ids=[PEER, OWN]))
        self.bridge.dispatch('renameHost', dict(host=PEER, name='My server'))
        self.bridge.snapshot()
        self.bridge.refresh(PEER)
        self.assertEqual(self.bridge.snapshot()['hosts'][PEER]['state'], 'available')
        next_launch = module.Bridge(self.config)
        snapshot = next_launch.snapshot()
        self.assertEqual(snapshot['order']['machines'], [PEER, OWN])
        self.assertEqual(snapshot['hosts'][PEER]['name'], 'My server')
        self.assertEqual(snapshot['hosts'][PEER]['state'], 'unverified')
        self.assertEqual(snapshot['hosts'][PEER]['catalog']['services'][0]['incarnation'], INCARNATION)
        self.assertEqual(next_launch.path.stat().st_mode & 0o077, 0)
        self.assertEqual(snapshot['telemetry']['peers'][PEER]['path'], 'direct')

    def test_imports_tauri_private_names_and_order_read_only(self):
        import sqlite3
        path = self.root / 'data/app.wispnet.desktop/localstorage/tauri_localhost_0.localstorage'
        path.parent.mkdir(parents=True)
        with sqlite3.connect(path) as db:
            db.execute('CREATE TABLE ItemTable (key TEXT, value BLOB)')
            db.executemany('INSERT INTO ItemTable VALUES (?, ?)', [
                ('peerNames', json.dumps({PEER: 'Tauri name'}).encode('utf-16-le')),
                ('cardOrder', json.dumps({'machines': [PEER, OWN]}).encode('utf-16-le')),
            ])
        db.close()
        original = path.read_bytes()
        migrated = module.Bridge(self.config)
        self.assertEqual(migrated.preferences['names'][PEER], 'Tauri name')
        self.assertEqual(migrated.preferences['order']['machines'], [PEER, OWN])
        self.assertEqual(path.read_bytes(), original)

    def test_cached_open_always_checks_owner_and_propagates_rejection(self):
        self.bridge.snapshot()
        self.bridge.refresh(PEER)
        with patch.object(self.bridge.api, 'request', side_effect=[{}, module.NodeRejected('Access denied')]) as request:
            with self.assertRaisesRegex(ValueError, 'Access denied'):
                self.bridge.dispatch('open', dict(host=PEER, service='web', incarnation=INCARNATION))
            self.assertEqual(request.call_args_list[-1].kwargs['incarnation'], INCARNATION)
            self.assertEqual(request.call_args_list[-1].kwargs['op'], 'open')

    def test_offline_snapshot_keeps_display_but_permission_errors_are_not_offline(self):
        self.bridge.snapshot()
        with patch.object(self.bridge.api, 'request', side_effect=FileNotFoundError(2, 'offline')):
            self.assertFalse(self.bridge.snapshot()['enabled'])
            self.assertEqual(self.bridge.snapshot()['peer'], OWN)
        with patch.object(self.bridge.api, 'request', side_effect=ValueError('Node socket must be owner-only')):
            with self.assertRaises(ValueError): self.bridge.snapshot()

    def test_settings_preserve_other_routes_and_remove_automatic_override(self):
        self.bridge.dispatch('saveSettings', dict(host=PEER, service='web', port=19000))
        saved = self.calls[-1]
        self.assertEqual(saved['op'], 'configure_access')
        self.assertEqual(saved['settings']['registry_port'], 18355)
        self.assertEqual(len(saved['settings']['service_ports']), 2)
        self.bridge.dispatch('saveSettings', dict(host=PEER, service='web', port=0))
        self.assertEqual(len(self.calls[-1]['settings']['service_ports']), 1)
        with self.assertRaises(ValueError): self.bridge.dispatch('saveSettings', dict(host=PEER, service='web', port=80))

    def test_unpair_uses_current_revision_and_never_removes_local_machine(self):
        self.bridge.snapshot()
        self.bridge.refresh(PEER)
        self.bridge.dispatch('removeHost', dict(host=PEER))
        self.assertIn(dict(op='unpair', host=PEER, expected_revision=7), self.calls)
        self.assertIsNone(self.bridge.cache.load(PEER))
        with self.assertRaises(ValueError): self.bridge.dispatch('removeHost', dict(host=OWN))

    def test_handoff_only_accepts_token_free_localhost_root(self):
        self.assertEqual(module.browser_url('http://web.localhost:18000/'), 'http://web.localhost:18000/')
        for url in ['https://web.localhost:18000/', 'http://web.localhost:18000/?token=x', 'http://evil.test/', 'http://web.localhost:18000/path', 'http://user@web.localhost:18000/']:
            with self.assertRaises(ValueError): module.browser_url(url)

    def test_explicit_owner_revision_and_no_arbitrary_helper_commands(self):
        operation = dict(op='revoke', peer=PEER, expected_revision=4)
        self.bridge.dispatch('desktopRequest', dict(payload=dict(op='mutate', operation=operation)))
        self.assertEqual(self.calls[-1]['operation'], operation)
        for method, args in [('desktopRequest', dict(payload=dict(op='shell'))), ('desktopData', dict(op='shell')), ('setAccess', dict(enabled=False))]:
            with self.assertRaises(ValueError): self.bridge.dispatch(method, args)



class NativeBridgeTest(unittest.TestCase):
    @unittest.skipUnless((module.ROOT / 'target/debug/wispnet').exists(), 'Build the node first')
    def test_real_node_local_catalog_owner_edit_and_fresh_browser_route(self):
        import http.client
        import http.server
        import socket
        import subprocess
        import threading
        import time
        from urllib.parse import urlsplit
        binary = module.ROOT / 'target/debug/wispnet'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config, state = root / 'host.toml', root / 'state'
            config.write_text('schema_version=2\ndevice_name="Fixture"\nstate_dir=' + json.dumps(str(state)) + '\nlog_level="info"\n[transport]\nbind_addr="127.0.0.1:0"\nrelay_url="https://relay.example"\n')
            subprocess.run([binary, 'host-init', '--config', config], check=True, capture_output=True, timeout=15)
            process = subprocess.Popen([binary, 'agent', '--config', config], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            class Handler(http.server.BaseHTTPRequestHandler):
                def do_GET(self):
                    self.send_response(200); self.end_headers(); self.wfile.write(b'flutter-native-route-ok')
                def log_message(self, *args): pass
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
            thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
            try:
                deadline = time.monotonic() + 15
                while not (state / 'node.sock').exists():
                    self.assertIsNone(process.poll()); self.assertLess(time.monotonic(), deadline); time.sleep(.05)
                with patch.dict(os.environ, XDG_DATA_HOME=str(root / 'data'), XDG_CONFIG_HOME=str(root / 'config')):
                    bridge = module.Bridge(config)
                initial = bridge.snapshot()
                self.assertTrue(initial['enabled'])
                own = initial['peer']
                self.assertTrue(initial['hosts'][own]['local'])
                status = bridge.dispatch('desktopStatus', {})
                bridge.dispatch('desktopRequest', {'payload': {'op': 'mutate', 'operation': {'op': 'service', 'id': 'fixture', 'display_name': 'Flutter fixture', 'kind': 'http', 'backend': f'127.0.0.1:{server.server_port}', 'allow': [], 'expected_revision': status['host']['revision']}}})
                with socket.socket() as available:
                    available.bind(('127.0.0.1', 0)); port = available.getsockname()[1]
                bridge.dispatch('saveSettings', dict(host=None, name='Fixture', port=port))
                service = bridge.snapshot()['hosts'][own]['catalog']['services'][0]
                handoff = bridge.dispatch('open', dict(host=own, service='fixture', incarnation=service['incarnation']))
                self.assertEqual(set(handoff), {'url'})
                url = urlsplit(handoff['url'])
                self.assertFalse(url.fragment)
                connection = http.client.HTTPConnection('127.0.0.1', url.port, timeout=10)
                connection.request('GET', '/', headers={'Host': url.netloc})
                response = connection.getresponse()
                self.assertEqual(response.status, 200)
                self.assertEqual(response.read(), b'flutter-native-route-ok')
                connection.close()
                with self.assertRaises(ValueError): bridge.dispatch('open', dict(host=own, service='fixture', incarnation='f' * 64))
                self.assertEqual(bridge.snapshot()['peer'], own)
                process.terminate(); process.wait(timeout=10)
                self.assertFalse(bridge.snapshot()['enabled'])
                self.assertEqual(bridge.snapshot()['peer'], own)
            finally:
                if process.poll() is None: process.terminate(); process.wait(timeout=10)
                server.shutdown(); server.server_close()

if __name__ == '__main__': unittest.main()
