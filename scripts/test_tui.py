"""Real PTY/native-node management: no user identities, grants, or services touched."""
import base64
import errno
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import socket
import struct
import subprocess
import termios
import time
import unittest
import pyte
from PIL import Image
import zxingcpp
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target/debug/wispnet'
spec = importlib.util.spec_from_file_location('tui_node_api', ROOT / 'apps/node/desktop/wispnet_desktop/api.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class Terminal:
    def __init__(self, config, width=140, height=75, monochrome=True):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            env = dict(os.environ, TERM='xterm-256color')
            if monochrome: env['NO_COLOR'] = '1'
            else: env.pop('NO_COLOR', None)
            env.pop('DISPLAY', None)
            env.pop('WAYLAND_DISPLAY', None)
            os.execve(BINARY, [str(BINARY), '--config', str(config)], env)
        self.before = termios.tcgetattr(self.fd)
        self.raw = bytearray()
        self.screen = pyte.Screen(width, height)
        self.stream = pyte.ByteStream(self.screen)
        self.resize(width, height)
        self.exited = False

    def resize(self, width, height):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack('HHHH', height, width, 0, 0))
        self.screen.resize(height, width)
        os.kill(self.pid, signal.SIGWINCH)
        if hasattr(self, 'exited'):
            # Wait for the application's redraw at the new bottom row before
            # sending another key; resizing the emulator alone is not an ack.
            deadline = time.monotonic() + 12
            while not re.search(r'v[0-9]+\.[0-9]+\.', self.screen.display[-1]):
                self.read()
                if time.monotonic() >= deadline:
                    raise AssertionError(f'Terminal resize was not redrawn at {width}x{height}')

    def read(self, seconds=.08):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if not select.select([self.fd], [], [], min(.03, max(0, deadline-time.monotonic())))[0]:
                continue
            try:
                data = os.read(self.fd, 262144)
            except OSError as error:
                if error.errno == errno.EIO: return
                raise
            if not data: return
            self.raw.extend(data)
            self.stream.feed(data)

    @property
    def text(self):
        return '\n'.join(self.screen.display)

    def expect(self, text, timeout=12):
        deadline = time.monotonic() + timeout
        while text not in self.text:
            self.read()
            if time.monotonic() >= deadline:
                raise AssertionError(f'Terminal never showed {text!r}:\n{self.text}')
        self.read(.08)

    def send(self, data):
        os.write(self.fd, data if isinstance(data, bytes) else data.encode())
        self.read(.12)

    def close(self, external=False):
        if self.exited: return
        if external: os.kill(self.pid, signal.SIGTERM)
        else: self.send(b'\x03')
        deadline = time.monotonic() + 8
        while True:
            self.read(.05)
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.exited = True
                if status != 0: raise AssertionError(f'TUI failed, status={status}: {self.text}')
                break
            if time.monotonic() > deadline:
                os.kill(self.pid, signal.SIGKILL)
                os.waitpid(self.pid, 0)
                raise AssertionError('TUI did not exit promptly')
        after = termios.tcgetattr(self.fd)
        assert after[3] & (termios.ICANON | termios.ECHO) == self.before[3] & (termios.ICANON | termios.ECHO), 'Terminal raw mode was not restored'
        assert b'\x1b[?1049l' in self.raw, 'Alternate screen was not restored'
        os.close(self.fd)

class TerminalManagement(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='wispnet-tui-test-')
        self.root = Path(self.temp.name)
        self.processes = []
        self.terminals = []

    def tearDown(self):
        try:
            for term in self.terminals:
                term.close()
        finally:
            for process in self.processes:
                process.terminate()
                process.wait(timeout=10)
            self.temp.cleanup()

    def host(self, name):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.bind(('127.0.0.1', 0))
            port = udp.getsockname()[1]
        config = self.root / (name + '.toml')
        state = self.root / name
        config.write_text(f'schema_version=2\ndevice_name="{name}"\nstate_dir="{state}"\nlog_level="info"\n[transport]\nbind_addr="127.0.0.1:{port}"\nrelay_url="https://relay.example"\n')
        result = subprocess.check_output([BINARY, 'host-init', '--config', config], text=True)
        peer = result.split('Endpoint ID: ')[1].strip()
        daemon = subprocess.Popen([BINARY, 'agent', '--config', config], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.processes.append(daemon)
        deadline = time.monotonic() + 10
        while not (state / 'node.sock').exists():
            self.assertIsNone(daemon.poll())
            self.assertLess(time.monotonic(), deadline)
            time.sleep(.03)
        api = module.NodeAPI(config)
        # Terminal Open enables browser access; never compete with the live node.
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as registry:
            registry.bind(('127.0.0.1', 0))
            registry_port = registry.getsockname()[1]
        api.request(op='configure_access', settings={
            'registry_port': registry_port, 'allowed_origins': [], 'service_ports': [],
        })
        return config, api, peer, port

    def terminal(self, config):
        term = Terminal(config)
        self.terminals.append(term)
        term.expect('ONLINE')
        return term

    def changed(self, api, condition):
        deadline = time.monotonic() + 12
        while True:
            state = api.request(op='status')
            if condition(state): return state
            self.assertLess(time.monotonic(), deadline, 'Expected policy change not observed')
            time.sleep(.04)

    def test_real_approval_grants_pairing_removal_and_qr(self):
        a, aapi, pa, porta = self.host('fixture-phone')
        b, bapi, pb, portb = self.host('fixture-host')
        for revision, sid in enumerate(['private', 'scan-only']):
            bapi.request(op='mutate', operation={'op':'service','expected_revision':revision,
                         'id':sid,'display_name':'Scan service' if sid=='scan-only' else 'Private service',
                         'kind':'http','backend':'127.0.0.1:8201','allow':[]})
        invitation_file = self.root / 'initial-invitation.json'
        subprocess.run([BINARY, 'host-invite', '--config', b, '--output', invitation_file], capture_output=True, check=True)
        invitation = json.loads(invitation_file.read_text())
        invitation['direct_hint'] = f'127.0.0.1:{portb}'
        aapi.request(op='pair', host=pb, invitation=json.dumps(invitation))
        term = self.terminal(b)
        term.send('3')
        term.expect('fixture-phone')
        term.send(b'\r')
        term.expect('Review pairing request')
        self.assertIn(pa, term.text)
        self.assertEqual(bapi.request(op='status')['host']['clients'], [])
        term.send(b'\r')
        term.expect('Approve device and select services')
        term.send(b' \x1b')
        self.assertEqual(bapi.request(op='status')['host']['clients'], [])
        self.assertTrue(all(not s['allow'] for s in bapi.request(op='status')['host']['services']))
        term.send(b'\r\r\x1b[B \x1b[B\r')
        term.expect('Approve pairing and service access')
        term.send(b'\r')
        approved = self.changed(bapi, lambda s: len(s['host']['clients']) == 1)
        self.assertEqual(approved['host']['revision'], 3)
        self.assertEqual({s['id']:s['allow'] for s in approved['host']['services']},
                         {'private':[], 'scan-only':[pa]})
        term.expect('1 clients')

        # The real form creates a loopback service with its explicitly selected device.
        term.send('5')
        term.send(b'\r')
        term.expect('Add service')
        term.send(b'Dashboard\t8310\t\t\t\r')
        term.expect('Save service and grants')
        self.assertIn(pa, term.text)
        term.send(b'\r')
        state = self.changed(bapi, lambda s: len(s['host']['services']) == 3)
        self.assertEqual(state['host']['services'][0]['allow'], [pa])
        term.expect('3 services')
        term.send(b'\x1b[B\r\x1b[B\r')
        term.expect('Browser service')
        self.assertTrue(bapi.request(op='status')['access_enabled'])
        self.assertIn('.localhost:', term.text)
        term.send(b'\r\r\r')
        term.expect('Edit service and device grants')
        term.send(b'\t\t\t \t\r')
        term.expect('Save service and grants')
        term.send(b'\r')
        self.changed(bapi, lambda s: s['host']['services'][0]['allow'] == [])
        term.expect('0 permitted devices')

        # Remove a specific approved client, using the same confirmation as the desktop.
        term.send('4')
        term.send(b'\r\x1b[B\r')
        term.expect('Remove this machine')
        self.assertIn(pa, term.text)
        term.send(b'\x1b')
        self.assertEqual(len(bapi.request(op='status')['host']['clients']), 1)
        term.send(b'\r\x1b[B\r\r')
        self.changed(bapi, lambda s: s['host']['clients'] == [])
        term.expect('0 clients')

        # Paste a code, review, and explicitly request pairing to the other native endpoint.
        fresh = self.root / 'remote-invitation.json'
        subprocess.run([BINARY, 'host-invite', '--config', a, '--output', fresh], capture_output=True, check=True)
        document = json.loads(fresh.read_text())
        document['direct_hint'] = f'127.0.0.1:{porta}'
        fresh.write_text(json.dumps(document))
        term.send('6')
        term.send(b'\r')
        term.expect('Paste pairing code')
        code = 'wispnet1:' + base64.urlsafe_b64encode(json.dumps(document).encode()).decode().rstrip('=')
        term.send(code)
        term.send(b'\t\r')
        term.expect('Request pairing with this host')
        self.assertIn(pa, term.text)
        self.assertNotIn(pa, bapi.request(op='status')['peer_pins'])
        term.send(b'\r')
        self.changed(bapi, lambda s: pa in s['peer_pins'])
        self.assertTrue(any(i['peer'] == pb and i['state'] == 'pending' for i in aapi.request(op='status')['invitations']))
        term.expect('Request sent')
        term.send('4')
        term.send(b'\r\x1b[B\r')
        term.expect('Remove this machine')
        term.send(b'\r')
        self.changed(bapi, lambda s: pa not in s['peer_pins'])
        term.expect('Saved')

        # Generate and copy inside the TUI, including on a terminal too small for QR.
        term.send('2')
        term.expect('Pairing code · c: copy')
        term.expect('wispnet1:')
        term.send('c')
        copies = re.findall(rb'\x1b]52;c;([A-Za-z0-9+/=]+)\x1b\\', bytes(term.raw))
        self.assertTrue(copies, 'TUI did not emit a complete clipboard command')
        copied = base64.b64decode(copies[-1]).decode()
        self.assertEqual(bapi.request(op='preview_invitation', invitation=copied)['host'], pb)
        term.resize(80, 24)
        term.expect('wispnet1:')
        term.send('c')
        copies = re.findall(rb'\x1b]52;c;([A-Za-z0-9+/=]+)\x1b\\', bytes(term.raw))
        self.assertEqual(base64.b64decode(copies[-1]).decode(), copied)
        term.resize(140, 75)

        # Native TUI QR is independently decoded from the terminal's actual styled cells.
        term.send('g')
        term.expect('In Wispnet on your phone')
        term.expect('Expires in')
        rows = []
        pixels = {' ': (False, False), '▀': (True, False), '▄': (False, True), '█': (True, True)}
        for y in range(term.screen.lines):
            cells = term.screen.buffer[y]
            positions = [x for x in range(term.screen.columns) if cells[x].bg in ('white','brightwhite','ffffff') and cells[x].fg in ('black','000000')]
            if len(positions) < 21: continue
            values = [pixels[cells[x].data] for x in positions]
            rows.extend([[v[0] for v in values], [v[1] for v in values]])
        self.assertTrue(rows, 'No QR raster found in terminal')
        size = len(rows[0])
        image = Image.new('L', (size, size))
        image.putdata([0 if dark else 255 for row in rows[:size] for dark in row])
        decoded = zxingcpp.read_barcode(image.resize((size*6, size*6), Image.Resampling.NEAREST))
        self.assertIsNotNone(decoded)
        code = decoded.text[9:]
        invitation = json.loads(base64.urlsafe_b64decode(code + '=' * (-len(code) % 4)))
        self.assertEqual(invitation['host'], pb)
        term.resize(80, 24)
        term.expect('Enlarge the terminal')
        term.resize(140, 75)
        term.expect('Expires in')
        term.close(external=True)

    def test_stale_revision_is_rejected_without_retry_and_mouse_can_navigate(self):
        config, api, _, _ = self.host('revision-fixture')
        term = self.terminal(config)
        # Actual SGR mouse event opens the fifth sidebar item.
        term.send(b'\x1b[<0;8;10M\x1b[<0;8;10m')
        term.expect('+ Add local service')
        term.send(b'\r')
        term.expect('Add service')
        term.send(b'Dashboard\t8310\t\t\r')  # No clients: Save follows Protocol.
        term.expect('Save service and grants')
        peer = subprocess.check_output([BINARY,'identity-init','--config',self.make_client()], text=True).split('Endpoint ID: ')[1].strip()
        api.request(op='mutate', operation={'op':'approve','peer':peer,'name':'Concurrent client','expected_revision':0})
        term.send(b'\r')
        term.expect('Operation not completed')
        self.assertEqual(api.request(op='status')['host']['services'], [])
        self.assertEqual(api.request(op='status')['host']['revision'], 1)
        term.close()

    def make_client(self):
        config = self.root / 'concurrent.toml'
        config.write_text(f'schema_version=1\ndevice_name="concurrent"\nstate_dir="{self.root / "concurrent"}"\nlog_level="info"\n')
        return config

if __name__ == '__main__': unittest.main()
