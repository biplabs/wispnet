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

if __name__=='__main__':unittest.main()
