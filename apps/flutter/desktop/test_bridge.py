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
                           host_name='Desktop', access_enabled=True, peer_pins=[PEER], invitations=[], relay='connected', peer_telemetry={PEER: dict(path='direct', rtt_ms=15)})
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

    def test_desktop_start_and_exit_control_node_and_propagate_stop_failure(self):
        with patch.object(self.bridge, 'lifecycle') as lifecycle:
            self.bridge.dispatch('startDesktop', {})
            lifecycle.assert_called_once_with(True)
            self.assertEqual(self.calls[-1], {'op': 'enable'})
            self.calls.clear()
            lifecycle.reset_mock()
            self.bridge.dispatch('setAccess', {'enabled': False})
            lifecycle.assert_called_once_with(False)
            lifecycle.side_effect = ValueError('Cannot stop node')
            with self.assertRaisesRegex(ValueError, 'Cannot stop node'):
                self.bridge.dispatch('setAccess', {'enabled': False})
        self.assertNotIn({'op': 'enable'}, self.calls)

    def test_shell_launch_uses_authoritative_kind_incarnation_and_pinned_cli_arguments(self):
        self.status['host']['services'] = [dict(id='shell', kind='shell', incarnation=INCARNATION)]
        args = dict(host=OWN, service='shell', incarnation=INCARNATION, session='main')
        with patch.object(module, 'launch_terminal') as launch:
            self.assertEqual(self.bridge.dispatch('openShell', args), {})
            command, state = launch.call_args.args
            self.assertEqual(command[1:], ['shell', '--host', OWN, '--service', 'shell',
                '--incarnation', INCARNATION, '--session', 'main', '--config', str(self.config.resolve())])
            self.assertEqual(state, self.state)
            self.assertNotIn('shell_open', [c['op'] for c in self.calls])
            launch.reset_mock()
            for invalid in [dict(args, incarnation='f' * 64), dict(args, service='web'),
                            dict(args, session='main; touch /tmp/injected')]:
                with self.assertRaises(ValueError):
                    self.bridge.dispatch('openShell', invalid)
            launch.assert_not_called()
            self.status['host']['services'][0]['kind'] = 'http'
            with self.assertRaises(ValueError):
                self.bridge.dispatch('openShell', args)

    def test_in_app_shell_session_operations_forward_to_node_api(self):
        self.calls.clear()
        self.bridge.dispatch('shellOpen', dict(host=OWN, service='shell', incarnation=INCARNATION, session='main', cols=100, rows=30))
        self.assertEqual(self.calls[-2:], [
            {'op': 'enable'},
            {'op': 'shell_open', 'host': OWN, 'service': 'shell', 'incarnation': INCARNATION, 'session': 'main', 'cols': 100, 'rows': 30},
        ])
        self.bridge.dispatch('shellPoll', dict(session='s-1'))
        self.assertEqual(self.calls[-1], {'op': 'shell_poll', 'session': 's-1'})
        self.bridge.dispatch('shellInput', dict(session='s-1', data_b64='bHM='))
        self.assertEqual(self.calls[-1], {'op': 'shell_input', 'session': 's-1', 'data_b64': 'bHM='})
        self.bridge.dispatch('shellResize', dict(session='s-1', cols=120, rows=40))
        self.assertEqual(self.calls[-1], {'op': 'shell_resize', 'session': 's-1', 'cols': 120, 'rows': 40})
        self.bridge.dispatch('shellClose', dict(session='s-1', terminate=True))
        self.assertEqual(self.calls[-1], {'op': 'shell_close', 'session': 's-1', 'terminate': True})
        self.bridge.dispatch('shellClose', dict(session='s-1'))
        self.assertEqual(self.calls[-1], {'op': 'shell_close', 'session': 's-1', 'terminate': False})

    def test_terminal_launch_keeps_cli_options_out_of_terminal_arguments(self):
        command = ['/tmp/Wispnet CLI', 'shell', '--session', 'main', '--config', '/tmp/with spaces/host.toml']
        with patch.object(module.sys, 'platform', 'linux'), patch.object(module.shutil, 'which',
             side_effect=lambda name: '/usr/bin/gnome-terminal' if name == 'gnome-terminal' else None), \
             patch.object(module.subprocess, 'Popen') as popen:
            module.launch_terminal(command, self.state)
            terminal_command = popen.call_args.args[0]
            self.assertEqual(terminal_command[:-1], ['/usr/bin/gnome-terminal', '--', '/bin/sh'])
            self.assertNotIn('--session', terminal_command)
            script = Path(terminal_command[-1])
            self.assertEqual(script.stat().st_mode & 0o777, 0o700)
            self.assertEqual(script.parent, self.state)
            self.assertEqual(module.shlex.split(script.read_text().splitlines()[-1])[1:], command)
            self.assertNotIn('shell', popen.call_args.kwargs)

    def test_terminal_script_preserves_literal_arguments_and_removes_itself(self):
        captured = self.root / 'captured.json'
        injected = self.root / 'injected'
        arguments = ['with spaces', 'single\'quote', 'semi; colon', f'$(touch {injected})', '--session', 'main']
        command = [module.sys.executable, '-c',
                   'import json,sys; open(sys.argv[1], "w").write(json.dumps(sys.argv[2:]))',
                   str(captured), *arguments]
        with patch.object(module.sys, 'platform', 'linux'), patch.object(module.shutil, 'which',
             side_effect=lambda name: '/usr/bin/konsole' if name == 'konsole' else None), \
             patch.object(module.subprocess, 'Popen') as popen:
            module.launch_terminal(command, self.state)
            terminal_command = popen.call_args.args[0]
        self.assertEqual(terminal_command[:-1], ['/usr/bin/konsole', '-e', '/bin/sh'])
        script = Path(terminal_command[-1])
        module.subprocess.run(terminal_command[-2:], check=True, timeout=5)
        self.assertEqual(json.loads(captured.read_text()), arguments)
        self.assertFalse(injected.exists())
        self.assertFalse(script.exists())

    def test_terminal_launch_failure_removes_private_script(self):
        with patch.object(module.sys, 'platform', 'linux'), patch.object(module.shutil, 'which',
             return_value='/usr/bin/konsole'), \
             patch.object(module.subprocess, 'Popen', side_effect=OSError('Cannot launch terminal')):
            with self.assertRaisesRegex(OSError, 'Cannot launch terminal'):
                module.launch_terminal(['wispnet', 'shell'], self.state)
        self.assertEqual(list(self.state.glob('shell-*.command')), [])
        with patch.object(module.sys, 'platform', 'linux'), patch.object(module.shutil, 'which', return_value=None):
            with self.assertRaisesRegex(ValueError, 'Install a terminal'):
                module.launch_terminal(['wispnet', 'shell'], self.state)
        self.assertEqual(list(self.state.glob('shell-*.command')), [])

    def test_macos_terminal_script_is_private_and_quotes_every_argument(self):
        command = ['/tmp/Wispnet CLI', 'shell', '--config', '/tmp/$(touch injected); quote\'/host.toml']
        with patch.object(module.sys, 'platform', 'darwin'), patch.object(module.subprocess, 'run') as run:
            module.launch_terminal(command, self.state)
        script = Path(run.call_args.args[0][-1])
        self.assertEqual(script.stat().st_mode & 0o777, 0o700)
        self.assertEqual(module.shlex.split(script.read_text().splitlines()[-1])[1:], command)
        self.assertEqual(script.parent, self.state)

    def test_online_requires_saved_listeners_and_recovers_after_node_restart(self):
        self.status['access_enabled'] = False
        self.assertTrue(self.bridge.snapshot()['enabled'])
        self.assertEqual(self.calls[-1], {'op': 'enable'})
        self.calls.clear()
        self.status['access_enabled'] = True
        self.assertTrue(self.bridge.snapshot()['enabled'])
        self.assertNotIn({'op': 'enable'}, self.calls)
        self.status['access_enabled'] = False
        original = self.bridge.api.request
        def unavailable(**args):
            if args['op'] == 'enable':
                raise module.NodeRejected('Saved browser port unavailable')
            return original(**args)
        with patch.object(self.bridge.api, 'request', side_effect=unavailable):
            with self.assertRaisesRegex(ValueError, 'Saved browser port unavailable'):
                self.bridge.snapshot()
            with patch.object(self.bridge, 'lifecycle'):
                with self.assertRaisesRegex(ValueError, 'Saved browser port unavailable'):
                    self.bridge.dispatch('startDesktop', {})

    def test_upgrade_preserves_private_order_names_and_cache_without_trusting_cache(self):
        self.bridge.dispatch('saveOrder', dict(group='machines', ids=[PEER, OWN]))
        self.bridge.preferences.setdefault('names', {})[PEER] = 'Obsolete alias'
        self.bridge.snapshot()
        self.bridge.refresh(PEER)
        self.assertEqual(self.bridge.snapshot()['hosts'][PEER]['state'], 'available')
        next_launch = module.Bridge(self.config)
        snapshot = next_launch.snapshot()
        self.assertEqual(snapshot['order']['machines'], [PEER, OWN])
        self.assertEqual(snapshot['hosts'][PEER]['name'], 'saved-host')
        self.assertEqual(snapshot['hosts'][PEER]['state'], 'unverified')
        self.assertEqual(snapshot['hosts'][PEER]['catalog']['services'][0]['incarnation'], INCARNATION)
        self.assertEqual(next_launch.path.stat().st_mode & 0o077, 0)
        self.assertEqual(snapshot['telemetry']['peers'][PEER]['path'], 'direct')

    def test_remote_id_is_owned_by_peer_and_old_alias_operation_is_unavailable(self):
        self.bridge.snapshot()
        self.status['host']['clients'][0]['name'] = 'Old device name'
        original = self.bridge.api.request
        def request(**args):
            if args['op'] == 'host_device_id': return {'device_id': 'canonical-device'}
            return original(**args)
        with patch.object(self.bridge.api, 'request', side_effect=request):
            self.bridge.refresh(PEER)
        self.assertEqual(self.bridge.snapshot()['hosts'][PEER]['name'], 'canonical-device')
        self.assertIn(dict(op='set_name', host=PEER, name='Desktop'), self.calls)
        with self.assertRaisesRegex(ValueError, 'Unsupported'):
            self.bridge.dispatch('renameHost', dict(host=PEER, name='alias'))
        self.assertNotIn('names', json.loads(self.bridge.path.read_text())['preferences'])

    def test_remote_public_links_are_optional_and_do_not_enter_catalog_baselines(self):
        self.bridge.snapshot()
        links = dict(version=1, host=PEER, services=[dict(service='web', incarnation=INCARNATION,
                    url='https://shared-web.wispnet.test/', state='online')])
        original = self.bridge.api.request
        def request(**args):
            return links if args['op'] == 'host_public_links' else original(**args)
        with patch.object(self.bridge.api, 'request', side_effect=request):
            self.bridge.refresh(PEER)
        row = self.bridge.snapshot()['hosts'][PEER]
        self.assertEqual(row['public_access'], links)
        self.assertEqual(row['catalog'], catalog())
        self.assertEqual(self.bridge.cache.load(PEER)['catalog'], catalog())
        def old_peer(**args):
            if args['op'] == 'host_public_links': raise ValueError('Unsupported request')
            return original(**args)
        with patch.object(self.bridge.api, 'request', side_effect=old_peer):
            self.bridge.refresh(PEER)
        row = self.bridge.snapshot()['hosts'][PEER]
        self.assertEqual(row['state'], 'available')
        self.assertNotIn('public_access', row)

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
        self.assertNotIn(PEER, migrated.preferences.get('names', {}))
        self.assertEqual(migrated.preferences['order']['machines'], [PEER, OWN])
        self.assertEqual(path.read_bytes(), original)

    def test_corrupt_display_cache_cannot_prevent_launch_or_fresh_refresh(self):
        self.bridge.snapshot()
        self.bridge.refresh(PEER)
        self.bridge.cache.path(PEER).write_text('{}')
        next_launch = module.Bridge(self.config)
        snapshot = next_launch.snapshot()
        self.assertEqual(snapshot['hosts'][PEER]['state'], 'unverified')
        self.assertEqual(snapshot['hosts'][PEER]['catalog']['services'], [])
        self.assertTrue(snapshot['hosts'][OWN]['local'])
        next_launch.refresh(PEER)
        self.assertEqual(next_launch.snapshot()['hosts'][PEER]['state'], 'available')

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

    def test_service_settings_change_the_shared_port_and_clear_legacy_overrides(self):
        self.bridge.dispatch('saveSettings', dict(host=PEER, service='web', port=19000))
        saved = self.calls[-1]
        self.assertEqual(saved['op'], 'configure_access')
        self.assertEqual(saved['settings']['registry_port'], 19000)
        self.assertEqual(saved['settings']['service_ports'], [])
        self.assertEqual(self.bridge.dispatch('settings', dict(host=PEER, service='other'))['port'], 19000)
        for port in [0, 80, 65536]:
            with self.assertRaises(ValueError):
                self.bridge.dispatch('saveSettings', dict(host=PEER, service='web', port=port))

    def test_unpair_uses_current_revision_and_never_removes_local_machine(self):
        self.bridge.snapshot()
        self.bridge.refresh(PEER)
        self.bridge.dispatch('removeHost', dict(host=PEER))
        self.assertIn(dict(op='unpair', host=PEER, expected_revision=7), self.calls)
        self.assertIsNone(self.bridge.cache.load(PEER))
        with self.assertRaises(ValueError): self.bridge.dispatch('removeHost', dict(host=OWN))

    def test_owned_service_public_state_and_authoritative_public_browser_handoff(self):
        self.status['host']['services'] = [dict(id='web', kind='http', incarnation=INCARNATION)]
        publication = dict(service='web', incarnation=INCARNATION, enabled=True,
                           state='online', url='https://web.wispnet.test/')
        self.status['public_access'] = dict(services=[publication])
        self.bridge.records[PEER] = dict(state='available', catalog=catalog())
        snapshot = self.bridge.snapshot()
        owned = snapshot['hosts'][OWN]['catalog']['services'][0]
        self.assertEqual(owned['public_access']['state'], 'online')
        self.assertNotIn('public_access', snapshot['hosts'][PEER]['catalog']['services'][0])
        publication['state'] = 'offline'
        self.assertEqual(self.bridge.snapshot()['hosts'][OWN]['catalog']['services'][0]['public_access']['state'], 'offline')
        with patch.object(module.subprocess, 'run') as launch:
            self.assertEqual(self.bridge.dispatch('launchPublic', dict(service='web', url='https://ignored.example/')), dict(url=publication['url']))
            self.assertEqual(launch.call_args.args[0][-1], publication['url'])
            original = publication['url']
            for invalid in ['http://web.wispnet.test/', 'https://user@web.wispnet.test/', 'https://web.wispnet.test/?token=x', 'https://web.wispnet.test:8443/']:
                publication['url'] = invalid
                with self.assertRaises(ValueError): self.bridge.dispatch('launchPublic', dict(service='web'))
            publication['url'] = original
            publication['enabled'] = False
            with self.assertRaises(ValueError): self.bridge.dispatch('launchPublic', dict(service='web'))
            self.assertEqual(launch.call_count, 1)
        publication['incarnation'] = 'e' * 64
        self.assertNotIn('public_access', self.bridge.snapshot()['hosts'][OWN]['catalog']['services'][0])

    def test_handoff_only_accepts_local_service_roots_and_registry_json(self):
        self.assertEqual(module.browser_url('http://web.localhost:18000/'), 'http://web.localhost:18000/')
        registry = 'http://wispnet.localhost:18400/registry'
        self.assertEqual(module.browser_url('http://registry.wispnet.localhost:18400/v1/services'), 'http://registry.wispnet.localhost:18400/v1/services')
        self.assertEqual(module.browser_url(registry), registry)
        for invalid in ['http://web.localhost:18400/v1/services', 'http://registry.wispnet.localhost:18400/v1/resolve', registry + '?token=x', registry + '#fragment']:
            with self.assertRaises(ValueError): module.browser_url(invalid)
        for url in ['https://web.localhost:18000/', 'http://web.localhost:18000/?token=x', 'http://evil.test/', 'http://web.localhost:18000/path', 'http://user@web.localhost:18000/']:
            with self.assertRaises(ValueError): module.browser_url(url)

    def test_generic_browser_handoff_accepts_public_https_roots_and_existing_local_urls(self):
        for platform, opener in [('linux', 'xdg-open'), ('darwin', '/usr/bin/open')]:
            for url in ['https://web.wispnet.test/', 'https://web.wispnet.test:443/',
                        'http://web.localhost:18000/', 'http://wispnet.localhost:18400/registry']:
                with self.subTest(platform=platform, url=url), \
                     patch.object(module.sys, 'platform', platform), \
                     patch.object(module.subprocess, 'run') as launch:
                    self.assertEqual(self.bridge.dispatch('launch', dict(url=url)), {})
                    launch.assert_called_once_with([opener, url], check=True, timeout=15,
                                                  stdout=module.subprocess.DEVNULL,
                                                  stderr=module.subprocess.DEVNULL)
                    self.assertNotIn('shell', launch.call_args.kwargs)

    def test_generic_browser_handoff_rejects_unsafe_urls_without_launching(self):
        invalid = [None, 123, 'http://web.wispnet.test/', 'https://user@web.wispnet.test/',
                   'https://user:password@web.wispnet.test/', 'https://@web.wispnet.test/',
                   'https://web.wispnet.test/?token=x', 'https://web.wispnet.test/?',
                   'https://web.wispnet.test/#fragment', 'https://web.wispnet.test/#',
                   'https://web.wispnet.test:8443/', 'https://web.wispnet.test:bad/',
                   'https://web.wispnet.test:/', 'https://web.wispnet.test',
                   'https://web.wispnet.test/path', 'https:///missing-host/',
                   'https://[broken/', 'https://bad..wispnet.test/',
                   'https://bad%20host.wispnet.test/', 'https://web.wispnet.test\\evil/',
                   'https://web.wispnet.test/\n', 'https://web.wispnet.test/\t',
                   'https://web.wispnet.test/\x7f', 'javascript:alert(1)',
                   'http://web.localhost:18000/path', 'http://web.localhost:18000/?token=x']
        with patch.object(module.subprocess, 'run') as launch:
            for url in invalid:
                with self.subTest(url=url), self.assertRaises(ValueError):
                    self.bridge.dispatch('launch', dict(url=url))
            launch.assert_not_called()

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
                with socket.socket() as available:
                    available.bind(('127.0.0.1', 0)); registry_port = available.getsockname()[1]
                bridge.api.request(op='configure_access', settings=dict(registry_port=registry_port, service_ports=[]))
                with patch.object(bridge, 'lifecycle'):
                    bridge.dispatch('startDesktop', {})
                initial = bridge.snapshot()
                self.assertTrue(initial['enabled'])
                own = initial['peer']
                self.assertTrue(initial['hosts'][own]['local'])
                status = bridge.dispatch('desktopStatus', {})
                bridge.dispatch('desktopRequest', {'payload': {'op': 'mutate', 'operation': {'op': 'service', 'id': 'fixture', 'display_name': 'Flutter fixture', 'kind': 'http', 'backend': f'127.0.0.1:{server.server_port}', 'allow': [], 'expected_revision': status['host']['revision']}}})
                with socket.socket() as available:
                    available.bind(('127.0.0.1', 0)); port = available.getsockname()[1]
                bridge.dispatch('saveSettings', dict(host=None, name='fixture', port=port))
                service = next(s for s in bridge.snapshot()['hosts'][own]['catalog']['services'] if s['id'] == 'fixture')
                handoff = bridge.dispatch('open', dict(host=own, service='fixture', incarnation=service['incarnation']))
                self.assertEqual(set(handoff), {'url'})
                url = urlsplit(handoff['url'])
                self.assertFalse(url.fragment)
                connection = http.client.HTTPConnection('127.0.0.1', url.port, timeout=10)
                connection.request('GET', '/', headers={'Host': url.netloc})
                response = connection.getresponse()
                self.assertEqual(response.status, 200)
                self.assertEqual(response.read(), b'flutter-native-route-ok')
                self.assertIsNone(response.getheader('Set-Cookie'))
                connection.close()
                # Restart the actual node, keep the desktop adapter alive, and
                # request the saved deep link without calling Open or the registry.
                process.terminate(); process.wait(timeout=10)
                process = subprocess.Popen([binary, 'agent', '--config', config], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                deadline = time.monotonic() + 15
                while True:
                    self.assertIsNone(process.poll()); self.assertLess(time.monotonic(), deadline)
                    try:
                        bridge.api.request(op='status')
                        break
                    except OSError:
                        time.sleep(.05)
                self.assertTrue(bridge.snapshot()['enabled'])
                for metadata in [
                    {'Sec-Fetch-Site': 'same-origin', 'Sec-Fetch-Mode': 'navigate', 'Sec-Fetch-Dest': 'document'},
                    {'Sec-Fetch-Site': 'cross-site', 'Sec-Fetch-Mode': 'navigate', 'Sec-Fetch-Dest': 'document'},
                    {'Sec-Fetch-Site': 'same-origin', 'Sec-Fetch-Mode': 'no-cors', 'Sec-Fetch-Dest': 'script', 'Cookie': '_wispnet_session=obsolete'},
                ]:
                    connection = http.client.HTTPConnection('127.0.0.1', url.port, timeout=10)
                    connection.request('GET', '/saved/path?x=1', headers={'Host': url.netloc, **metadata})
                    response = connection.getresponse()
                    self.assertEqual(response.status, 200)
                    self.assertEqual(response.read(), b'flutter-native-route-ok')
                    self.assertIsNone(response.getheader('Set-Cookie'))
                    connection.close()
                # The ordinary startup path also restores routes after offline.
                bridge.api.request(op='stop')
                with patch.object(bridge, 'lifecycle'):
                    bridge.dispatch('startDesktop', {})
                connection = http.client.HTTPConnection('127.0.0.1', url.port, timeout=10)
                connection.request('GET', '/', headers={'Host': url.netloc, 'Sec-Fetch-Site': 'same-origin'})
                response = connection.getresponse()
                self.assertEqual(response.status, 200); response.read(); connection.close()
                with self.assertRaises(ValueError): bridge.dispatch('open', dict(host=own, service='fixture', incarnation='f' * 64))
                self.assertEqual(bridge.snapshot()['peer'], own)
                process.terminate(); process.wait(timeout=10)
                self.assertFalse(bridge.snapshot()['enabled'])
                self.assertEqual(bridge.snapshot()['peer'], own)
            finally:
                if process.poll() is None: process.terminate(); process.wait(timeout=10)
                server.shutdown(); server.server_close()

if __name__ == '__main__': unittest.main()
