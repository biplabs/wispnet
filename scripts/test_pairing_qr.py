import base64
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from pairing_qr import render

class QrExportTests(unittest.TestCase):
    def test_private_json_and_code_png_no_overwrite_and_input_rejections(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'invite';output=root/'qr.png'
            document=json.dumps({'version':1,'host':'a'*64,'invitation':'b'*64,'code':'c'*64,'scope':'request_access','expires_at':int(time.time())+300,'relay_url':'https://relay.example'})
            source.write_text(document);source.chmod(0o600)
            render(source,output)
            self.assertTrue(output.read_bytes().startswith(b'\x89PNG\r\n\x1a\n'))
            self.assertEqual(output.stat().st_mode & 0o077,0)
            original=output.read_bytes()
            with self.assertRaises(FileExistsError):render(source,output)
            self.assertEqual(output.read_bytes(),original)
            output.unlink();source.write_text('wispnet1:'+base64.urlsafe_b64encode(document.encode()).decode().rstrip('='));render(source,output)
            self.assertEqual(output.read_bytes(),original)
            output.unlink();source.chmod(0o644)
            with self.assertRaises(ValueError):render(source,output)
            source.chmod(0o600);link=root/'link';link.symlink_to(source)
            with self.assertRaises(OSError):render(link,output)
            for invalid in ['not an invitation','x'*8193,document.replace('request_access','admin'),document.replace(str(json.loads(document)['expires_at']),'1')]:
                source.write_text(invalid)
                with self.assertRaises((ValueError,TypeError)):render(source,output)
                self.assertFalse(output.exists())


class NativeQrTests(unittest.TestCase):
    def test_terminal_and_private_node_api_encode_real_host_invitations(self):
        import importlib.util
        import subprocess
        import pty
        import fcntl
        import termios
        import struct
        import errno
        import zxingcpp
        from PIL import Image
        root_repo = Path(__file__).resolve().parents[1]
        binary = root_repo / 'target/debug/wispnet'
        spec = importlib.util.spec_from_file_location('qr_node_api', root_repo / 'apps/node/desktop/wispnet_desktop/api.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root / 'host.toml'
            state = root / 'state'
            config.write_text(f'schema_version=2\ndevice_name="qr-test"\nstate_dir="{state}"\nlog_level="info"\n[transport]\nbind_addr="127.0.0.1:0"\nrelay_url="https://relay.example"\n')
            subprocess.run([binary, 'host-init', '--config', config], check=True, capture_output=True)
            daemon = subprocess.Popen([binary, 'agent', '--config', config], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 10
                while not (state / 'node.sock').exists():
                    self.assertIsNone(daemon.poll())
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(.05)
                api = module.NodeAPI(config)
                before = api.request(op='status')['host']
                result = api.request(op='invite_qr')
                matrix = result['matrix']
                self.assertTrue(all(not cell for row in matrix[:4] for cell in row))
                self.assertTrue(all(not cell for row in matrix[-4:] for cell in row))

                def decode(matrix):
                    size = len(matrix)
                    image = Image.new('L', (size, size))
                    image.putdata([0 if cell else 255 for row in matrix for cell in row])
                    decoded = zxingcpp.read_barcode(image.resize((size * 6, size * 6), Image.Resampling.NEAREST))
                    self.assertIsNotNone(decoded)
                    self.assertTrue(decoded.text.startswith('wispnet1:'))
                    encoded = decoded.text[9:]
                    document = json.loads(base64.urlsafe_b64decode(encoded + '=' * (-len(encoded) % 4)))
                    self.assertEqual(document['host'], before['host'])
                    self.assertEqual(document['scope'], 'request_access')
                    self.assertTrue(document['host_name'])
                    self.assertGreater(document['expires_at'], time.time())
                    self.assertLessEqual(document['expires_at'], time.time() + 300)
                    return document

                document = decode(matrix)
                self.assertEqual(document['invitation'], result['invitation'])
                code_document = json.loads(base64.urlsafe_b64decode(result['code'][9:] + '=' * (-len(result['code'][9:]) % 4)))
                self.assertEqual(code_document, document)
                emitted = subprocess.run([binary, '--pairing-code', '--config', config], check=True, capture_output=True, text=True).stdout.strip()
                self.assertTrue(emitted.startswith('wispnet1:'))
                preview = api.request(op='preview_invitation', invitation=emitted)
                self.assertEqual(preview['host'], before['host'])
                master, slave = pty.openpty()
                try:
                    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 100, 200, 0, 0))
                    process = subprocess.Popen([binary, '--qrcode', '--config', config], stdout=slave, stderr=slave)
                    os.close(slave)
                    slave = None
                    chunks = bytearray()
                    while True:
                        try:
                            chunk = os.read(master, 65536)
                            if not chunk: break
                            chunks.extend(chunk)
                        except OSError as error:
                            if error.errno == errno.EIO: break
                            raise
                    self.assertEqual(process.wait(timeout=10), 0)
                finally:
                    os.close(master)
                    if slave is not None: os.close(slave)
                rows = []
                pixels = {' ': (False, False), '▀': (True, False), '▄': (False, True), '█': (True, True)}
                for line in chunks.decode().splitlines():
                    if not line.startswith('\x1b[30;107m'): continue
                    values = [pixels[c] for c in line.removeprefix('\x1b[30;107m').removesuffix('\x1b[0m')]
                    rows.extend([[p[0] for p in values], [p[1] for p in values]])
                self.assertTrue(rows)
                terminal_invite = decode(rows[:len(rows[0])])
                self.assertNotEqual(terminal_invite['invitation'], document['invitation'])
                after = api.request(op='status')
                self.assertEqual(after['host'], before)
                self.assertTrue(any(i['invitation'] == document['invitation'] and i['state'] == 'issued' for i in after['invitations']))
            finally:
                daemon.terminate()
                daemon.wait(timeout=10)

if __name__ == '__main__': unittest.main()
