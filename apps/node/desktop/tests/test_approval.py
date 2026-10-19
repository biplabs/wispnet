"""Approval review pins the request/revision and selects only explicit service grants."""
import time
import unittest
from pathlib import Path
from unittest.mock import patch
from PySide6.QtCore import Qt
from PySide6.QtWidgets import QApplication, QDialog, QListWidget, QLabel
from wispnet_desktop.app import Window, appearance

class API:
    def request(self, **request):
        raise OSError('fixture unavailable')

class ApprovalChecks(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])
        appearance(cls.app)

    def window(self, existing=False, services=True):
        window = Window(API(), tray=False)
        window.timer.stop()
        window.pool.waitForDone()
        QApplication.processEvents()
        window.updated({
            'api_version': 1, 'access_enabled': False, 'host_name': 'Approval fixture',
            'connected_peers': {}, 'recent_peers': {}, 'transport_streams': 0,
            'relay': 'connected', 'peer_pins': [],
            'invitations': [{'invitation': 'c'*64, 'peer': 'b'*64, 'name': 'Test phone',
                             'state': 'pending', 'expires_at': int(time.time())+300}],
            'host': {'host': 'a'*64, 'revision': 7,
                     'clients': [{'peer':'b'*64, 'name':'Test phone'}] if existing else [],
                     'services': [{'id':sid, 'display_name':sid.title(), 'kind':'http',
                                   'incarnation':sid[0]*64, 'backend':'127.0.0.1:8201',
                                   'allow':['b'*64] if existing and sid=='alpha' else []}
                                  for sid in ['alpha', 'beta']] if services else []},
        }, None)
        window.pending.selectRow(0)
        self.addCleanup(window.deleteLater)
        self.addCleanup(window.close)
        return window

    def test_selected_services_and_original_request_survive_polling(self):
        window = self.window()
        def approve(dialog):
            choices = dialog.findChild(QListWidget, 'approvalServices')
            self.assertEqual(choices.count(), 2)
            self.assertTrue(all(choices.item(i).checkState()==Qt.CheckState.Unchecked for i in range(2)))
            self.assertTrue(any('b'*64 in label.text() for label in dialog.findChildren(QLabel)))
            choices.item(1).setCheckState(Qt.CheckState.Checked)
            # A refresh may reorder requests and change policy while the dialog is open.
            window.snapshot['host']['revision'] = 8
            window.fill(window.pending, [['d'*64, 'Different phone', 'e'*64, 'pending']])
            return QDialog.DialogCode.Accepted
        with patch.object(QDialog,'exec',approve), patch.object(window,'perform') as perform:
            window.decide(True)
        operation = perform.call_args.args[0]['operation']
        self.assertEqual(operation, {'op':'approve_invitation','invitation':'c'*64,
                                     'expected_revision':7,'services':['beta']})

    def test_cancel_sends_nothing_and_empty_selection_is_pairing_only(self):
        window = self.window()
        with patch.object(QDialog,'exec',return_value=QDialog.DialogCode.Rejected), patch.object(window,'perform') as perform:
            window.decide(True)
            perform.assert_not_called()
        with patch.object(QDialog,'exec',return_value=QDialog.DialogCode.Accepted), patch.object(window,'perform') as perform:
            window.decide(True)
            self.assertEqual(perform.call_args.args[0]['operation']['services'], [])
        empty = self.window(services=False)
        with patch.object(QDialog,'exec',return_value=QDialog.DialogCode.Accepted), patch.object(empty,'perform') as perform:
            empty.decide(True)
            self.assertEqual(perform.call_args.args[0]['operation']['services'], [])

    def test_existing_access_is_visible_and_cannot_be_accidentally_unchecked(self):
        window = self.window(existing=True)
        def approve(dialog):
            choices = dialog.findChild(QListWidget,'approvalServices')
            self.assertEqual(choices.item(0).checkState(),Qt.CheckState.Checked)
            self.assertFalse(choices.item(0).flags() & Qt.ItemFlag.ItemIsEnabled)
            self.assertEqual(choices.item(1).checkState(),Qt.CheckState.Unchecked)
            return QDialog.DialogCode.Accepted
        with patch.object(QDialog,'exec',approve), patch.object(window,'perform') as perform:
            window.decide(True)
            self.assertEqual(perform.call_args.args[0]['operation']['services'], ['alpha'])

    def test_gui_approval_grants_selected_service_through_real_native_nodes(self):
        import json
        import socket
        import subprocess
        import tempfile
        from wispnet_desktop.api import NodeAPI
        import http.server
        import http.client
        import threading
        from urllib.parse import urlsplit
        class Backend(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'reverse-access-ok')
            def log_message(self,*args):
                pass
        backend = http.server.ThreadingHTTPServer(('127.0.0.1',0),Backend)
        thread = threading.Thread(target=backend.serve_forever,daemon=True)
        thread.start()
        self.addCleanup(thread.join,5)
        self.addCleanup(backend.server_close)
        self.addCleanup(backend.shutdown)
        binary = Path(__file__).resolve().parents[4] / 'target/debug/wispnet'
        processes = []
        window = None
        with tempfile.TemporaryDirectory(prefix='wispnet-gui-approval-') as temporary:
            root = Path(temporary)
            try:
                def host(name):
                    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
                        udp.bind(('127.0.0.1',0))
                        port = udp.getsockname()[1]
                    config = root / (name+'.toml')
                    state = root / name
                    config.write_text(f'schema_version=2\ndevice_name="{name}"\nstate_dir="{state}"\nlog_level="info"\n[transport]\nbind_addr="127.0.0.1:{port}"\nrelay_url="https://relay.example"\n')
                    output = subprocess.check_output([binary,'host-init','--config',config],text=True)
                    peer = output.split('Endpoint ID: ')[1].strip()
                    process = subprocess.Popen([binary,'agent','--config',config],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                    processes.append(process)
                    deadline = time.monotonic()+10
                    while not (state/'node.sock').exists():
                        self.assertIsNone(process.poll())
                        self.assertLess(time.monotonic(),deadline)
                        time.sleep(.02)
                    return config,NodeAPI(config),peer,port
                a,aapi,pa,_ = host('gui-phone')
                aapi.request(op='mutate',operation={'op':'service','expected_revision':0,
                             'id':'reverse','backend':f'127.0.0.1:{backend.server_port}','allow':[]})
                b,bapi,pb,portb = host('gui-host')
                for revision,sid in enumerate(['alpha','beta']):
                    bapi.request(op='mutate',operation={'op':'service','expected_revision':revision,
                                 'id':sid,'backend':'127.0.0.1:8201','allow':[]})
                invitation = root/'invite.json'
                subprocess.run([binary,'host-invite','--config',b,'--output',invitation],capture_output=True,check=True)
                document = json.loads(invitation.read_text())
                document['direct_hint'] = f'127.0.0.1:{portb}'
                aapi.request(op='pair',host=pb,invitation=json.dumps(document))
                with self.assertRaisesRegex(ValueError, "awaiting approval"):
                    aapi.request(op='catalog',host=pb)
                # Pending status comes from durable host state, including after restart.
                processes[-1].terminate()
                processes[-1].wait(timeout=10)
                process = subprocess.Popen([binary,'agent','--config',b],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                processes.append(process)
                deadline = time.monotonic()+10
                while True:
                    try:
                        bapi.request(op='status')
                        break
                    except (OSError,ValueError):
                        self.assertIsNone(process.poll())
                        self.assertLess(time.monotonic(),deadline)
                        time.sleep(.02)
                with self.assertRaisesRegex(ValueError, "awaiting approval"):
                    aapi.request(op='catalog',host=pb)
                window = Window(bapi,tray=False)
                window.timer.stop()
                window.pool.waitForDone()
                QApplication.processEvents()
                window.pending.selectRow(0)
                def choose(dialog):
                    choices = dialog.findChild(QListWidget,'approvalServices')
                    self.assertTrue(all(choices.item(i).checkState()==Qt.CheckState.Unchecked
                                        for i in range(choices.count())))
                    for i in range(choices.count()):
                        if choices.item(i).data(Qt.ItemDataRole.UserRole)=='beta':
                            choices.item(i).setCheckState(Qt.CheckState.Checked)
                    return QDialog.DialogCode.Accepted
                with patch.object(QDialog,'exec',choose):
                    window.decide(True)
                deadline = time.monotonic()+10
                while True:
                    QApplication.processEvents()
                    status = bapi.request(op='status')['host']
                    if status['clients']: break
                    self.assertLess(time.monotonic(),deadline)
                    time.sleep(.02)
                self.assertEqual(status['revision'],3)
                self.assertEqual(status['clients'][0]['peer'],pa)
                self.assertEqual({s['id']:s['allow'] for s in status['services']},
                                 {'alpha':[],'beta':[pa]})
                self.assertEqual([s['id'] for s in aapi.request(op='catalog',host=pb)['services']],['beta'])
                # One invitation introduces both hosts; reverse services stay denied.
                self.assertIn(pa,bapi.request(op='status')['peer_pins'])
                self.assertIn(pb,aapi.request(op='status')['peer_pins'])
                self.assertEqual(bapi.request(op='catalog',host=pa)['services'],[])
                self.assertEqual(aapi.request(op='status')['host']['services'][0]['allow'],[])
                aapi.request(op='mutate',operation={'op':'service','expected_revision':2,
                             'id':'reverse','backend':f'127.0.0.1:{backend.server_port}','allow':[pb]})
                self.assertEqual([s['id'] for s in bapi.request(op='catalog',host=pa)['services']],['reverse'])
                self.assertEqual([s['id'] for s in aapi.request(op='catalog',host=pb)['services']],['beta'])
                bapi.request(op='enable')
                service = bapi.request(op='catalog',host=pa)['services'][0]
                url = bapi.request(op='open',host=pa,service='reverse',incarnation=service['incarnation'])['url']
                parsed = urlsplit(url)
                def exchange(headers):
                    connection = http.client.HTTPConnection('127.0.0.1',parsed.port,timeout=5)
                    connection.request('GET','/',headers={'Host':parsed.netloc,**headers})
                    response = connection.getresponse()
                    result = response.status,dict(response.getheaders()),response.read()
                    connection.close()
                    return result
                status,headers,_ = exchange({'Sec-Fetch-Site':'none','Sec-Fetch-Mode':'navigate','Sec-Fetch-Dest':'document'})
                self.assertEqual(status,303)
                cookie = headers['Set-Cookie'].split(';')[0]
                self.assertEqual(exchange({'Cookie':cookie})[2],b'reverse-access-ok')
                aapi.request(op='mutate',operation={'op':'service','expected_revision':3,
                             'id':'reverse','backend':f'127.0.0.1:{backend.server_port}','allow':[]})
                self.assertEqual(bapi.request(op='catalog',host=pa)['services'],[])
                self.assertEqual(exchange({'Cookie':cookie})[0],403)
                aapi.request(op='mutate',operation={'op':'service','expected_revision':4,
                             'id':'reverse','backend':f'127.0.0.1:{backend.server_port}','allow':[pb]})
                # Both halves survive daemon restarts without another invitation.
                for process in processes:
                    process.terminate()
                    process.wait(timeout=10)
                processes.clear()
                for config in (a,b):
                    process = subprocess.Popen([binary,'agent','--config',config],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                    processes.append(process)
                deadline = time.monotonic()+10
                while True:
                    try:
                        self.assertIn(pa,bapi.request(op='status')['peer_pins'])
                        self.assertIn(pb,aapi.request(op='status')['peer_pins'])
                        break
                    except (OSError,ValueError):
                        self.assertLess(time.monotonic(),deadline)
                        time.sleep(.02)
                self.assertEqual([s['id'] for s in bapi.request(op='catalog',host=pa)['services']],['reverse'])
                # Local removal is immediate; the other host learns it on catalog contact.
                bapi.request(op='unpair',host=pa)
                removed = bapi.request(op='status')
                self.assertNotIn(pa,removed['peer_pins'])
                self.assertEqual(removed['host']['clients'],[])
                self.assertTrue(all(not service['allow'] for service in removed['host']['services']))
                with self.assertRaisesRegex(ValueError, 'Pairing removed'):
                    aapi.request(op='catalog',host=pb)
                self.assertEqual(aapi.request(op='status')['host']['services'][0]['allow'],[])
                self.assertNotIn(pb,aapi.request(op='status')['peer_pins'])
                with self.assertRaises(ValueError):
                    bapi.request(op='catalog',host=pa)

            finally:
                if window is not None:
                    window.pool.waitForDone()
                    window.peer_pool.waitForDone()
                    QApplication.processEvents()
                    window.close()
                    window.deleteLater()
                for process in processes:
                    process.terminate()
                    process.wait(timeout=10)

if __name__ == '__main__': unittest.main()
