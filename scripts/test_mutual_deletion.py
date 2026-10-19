"""Real native endpoints: removal learned on contact, offline safety, legacy pins and session isolation."""
import importlib.util
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target/debug/wispnet'
spec = importlib.util.spec_from_file_location('deletion_node_api', ROOT/'apps/node/desktop/wispnet_desktop/api.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class MutualDeletion(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='wispnet-delete-')
        self.root = Path(self.temporary.name)
        self.processes = {}
        self.addCleanup(self.temporary.cleanup)
        self.addCleanup(self.stop_all)

    def stop_all(self):
        for name in list(self.processes): self.stop(name)

    def stop(self, name):
        process = self.processes.pop(name)
        process.terminate()
        process.wait(timeout=10)

    def start(self, name):
        process = subprocess.Popen([BINARY,'agent','--config',self.root/(name+'.toml')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        self.processes[name] = process
        api = module.NodeAPI(self.root/(name+'.toml'))
        deadline = time.monotonic()+10
        while True:
            self.assertIsNone(process.poll())
            try:
                api.request(op='status')
                return api
            except (OSError,ValueError):
                self.assertLess(time.monotonic(),deadline)
                time.sleep(.02)

    def host(self, name):
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as udp:
            udp.bind(('127.0.0.1',0))
            port = udp.getsockname()[1]
        config = self.root/(name+'.toml')
        config.write_text(f'schema_version=2\ndevice_name="{name}"\nstate_dir="{self.root/name}"\nlog_level="info"\n[transport]\nbind_addr="127.0.0.1:{port}"\nrelay_url="https://relay.example"\n')
        output = subprocess.check_output([BINARY,'host-init','--config',config],text=True)
        peer = output.split('Endpoint ID: ')[1].strip()
        return self.start(name),peer,port

    def pair(self, a, b, pb, portb):
        invitation = self.root/f'invitation-{time.monotonic_ns()}.json'
        subprocess.run([BINARY,'host-invite','--config',b.config,'--output',invitation],capture_output=True,check=True)
        document = json.loads(invitation.read_text())
        document['direct_hint'] = f'127.0.0.1:{portb}'
        a.request(op='pair',host=pb,invitation=json.dumps(document))
        b.request(op='mutate',operation={'op':'approve_invitation','invitation':document['invitation'],
                                       'expected_revision':b.request(op='status')['host']['revision']})
        return document['invitation']

    def test_offline_removal_is_learned_on_contact_after_restarts(self):
        a,pa,_ = self.host('a')
        b,pb,portb = self.host('b')
        self.pair(a,b,pb,portb)
        for api,peer in [(a,pb),(b,pa)]:
            api.request(op='mutate',operation={'op':'service','id':'private','backend':'127.0.0.1:8201',
                'allow':[peer],'expected_revision':api.request(op='status')['host']['revision']})
        self.stop('b')
        before = a.request(op='status')['host']['revision']
        with self.assertRaises(ValueError):
            a.request(op='unpair',host=pb,expected_revision=before-1)
        self.assertEqual(len(a.request(op='status')['host']['clients']),1)
        a.request(op='unpair',host=pb,expected_revision=before)
        state = a.request(op='status')
        self.assertEqual(state['host']['clients'],[])
        self.assertEqual(state['peer_pins'],[])
        self.assertEqual(state['host']['services'][0]['allow'],[])
        self.assertNotIn('pending_deletions',state)
        # Local removal survives sender downtime; the recipient learns it only on contact.
        self.stop('a')
        a = self.start('a')
        self.assertNotIn('pending_deletions',a.request(op='status'))
        b = self.start('b')
        self.assertIn(pa,b.request(op='status')['peer_pins'])
        with self.assertRaisesRegex(ValueError,'Pairing removed'):
            b.request(op='catalog',host=pa)
        self.assertEqual(b.request(op='status')['host']['clients'],[])
        self.assertEqual(b.request(op='status')['peer_pins'],[])
        self.assertEqual(b.request(op='status')['host']['services'][0]['allow'],[])
        with self.assertRaises(ValueError): b.request(op='catalog',host=pa)
        self.stop('b')
        b = self.start('b')
        self.assertEqual(b.request(op='status')['peer_pins'],[])

    def test_fresh_pairing_supersedes_a_local_removal(self):
        a,pa,_ = self.host('a')
        b,pb,portb = self.host('b')
        old = self.pair(a,b,pb,portb)
        # A fresh handshake establishes a new session rather than inheriting removal.
        a.request(op='unpair',host=pb)
        new = self.pair(a,b,pb,portb)
        self.assertNotEqual(old,new)
        b.request(op='catalog',host=pa)
        self.assertIn(pa,b.request(op='status')['peer_pins'])
        self.assertIn(pb,a.request(op='status')['peer_pins'])
        self.assertEqual(a.request(op='catalog',host=pb)['services'],[])
        self.assertEqual(b.request(op='catalog',host=pa)['services'],[])

    def test_empty_catalog_pending_approval_and_timeout_keep_the_pairing(self):
        a,pa,_ = self.host('a')
        b,pb,portb = self.host('b')
        invitation = self.root/'pending.json'
        subprocess.run([BINARY,'host-invite','--config',b.config,'--output',invitation],capture_output=True,check=True)
        document = json.loads(invitation.read_text())
        document['direct_hint'] = f'127.0.0.1:{portb}'
        a.request(op='pair',host=pb,invitation=json.dumps(document))
        with self.assertRaises(ValueError): a.request(op='catalog',host=pb)
        self.assertIn(pb,a.request(op='status')['peer_pins'])
        b.request(op='mutate',operation={'op':'approve_invitation','expected_revision':0,'invitation':document['invitation']})
        self.assertEqual(a.request(op='catalog',host=pb)['services'],[])
        self.assertIn(pb,a.request(op='status')['peer_pins'])
        self.stop('b')
        with self.assertRaises((OSError,ValueError)): a.request(op='catalog',host=pb)
        self.assertIn(pb,a.request(op='status')['peer_pins'])
        self.assertEqual(a.request(op='status')['host']['clients'][0]['peer'],pb)

    def test_legacy_one_way_pin_is_removed_on_recipient(self):
        a,pa,porta = self.host('a')
        b,pb,portb = self.host('b')
        # Model a pre-mutual installation: A has B's pin, B has approved A.
        self.stop('a')
        pins = self.root/'a'/'peer-pins'
        pins.mkdir(mode=0o700)
        pin = pins/(pb+'.json')
        pin.write_text(json.dumps({'version':1,'host':pb,'relay_url':'https://relay.example/',
                                   'direct_hint':f'127.0.0.1:{portb}'}))
        pin.chmod(0o600)
        a = self.start('a')
        b.request(op='mutate',operation={'op':'approve','peer':pa,'name':'Legacy A','expected_revision':0})
        # Give B a protected route hint for this local fixture, as a relay would provide.
        self.stop('b')
        pins = self.root/'b'/'peer-pins'
        pins.mkdir(mode=0o700)
        pin = pins/(pa+'.json')
        pin.write_text(json.dumps({'version':1,'host':pa,'relay_url':'https://relay.example/',
                                   'direct_hint':f'127.0.0.1:{porta}'}))
        pin.chmod(0o600)
        b = self.start('b')
        b.request(op='unpair',host=pa)
        with self.assertRaisesRegex(ValueError,'Pairing removed'):
            a.request(op='catalog',host=pb)
        self.assertNotIn(pb,a.request(op='status')['peer_pins'])
        self.stop('a')
        a = self.start('a')
        self.assertNotIn(pb,a.request(op='status')['peer_pins'])

if __name__ == '__main__': unittest.main()
