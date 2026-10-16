#!/usr/bin/env python3
"""Standalone relay TLS, protected file, role separation and shutdown checks."""
import json
import os
from pathlib import Path
import socket
import ssl
import subprocess
import tempfile
import urllib.request
from relay_smoke import ROOT, BINARY, event, run, stop


def main():
    with tempfile.TemporaryDirectory(prefix='wipsnet-standalone-relay-') as temporary:
        root=Path(temporary);key=root/'key.pem';certificate=root/'certificate.pem'
        run(['openssl','req','-x509','-newkey','ed25519','-nodes','-days','1',
             '-keyout',key,'-out',certificate,'-subj','/CN=127.0.0.1',
             '-addext','subjectAltName=IP:127.0.0.1','-addext','basicConstraints=critical,CA:FALSE'])
        key.chmod(0o600)
        with socket.socket() as port_socket:
            port_socket.bind(('127.0.0.1',0));port=port_socket.getsockname()[1]
        config=root/'relay.toml'
        config.write_text(f"listen='127.0.0.1:{port}'\ncertificate='{certificate}'\nprivate_key='{key}'\nmax_connections=2\n")
        config.chmod(0o600)
        command=[ROOT/'target/debug/wipsnet-relay','--config',config]
        relay=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        try:
            started=event(relay,relay.stdout,'started');assert started['role']=='packet_relay'
            context=ssl.create_default_context(cafile=str(certificate))
            with urllib.request.urlopen(f'https://127.0.0.1:{port}/ping',context=context,timeout=5) as response:
                assert response.status==200
            try:
                urllib.request.urlopen(f'https://localhost:{port}/ping',context=context,timeout=5)
            except urllib.error.URLError: pass
            else: raise AssertionError('wrong TLS hostname was accepted')
            assert set(p.name for p in root.iterdir())=={'key.pem','certificate.pem','relay.toml'},'relay created node/application state'
            # A native Iroh client catches certificate-chain errors that a
            # Python/curl HTTPS health probe alone can miss.
            def node_config(label,schema):
                path=root/(label+'.toml')
                path.write_text(f"schema_version={schema}\ndevice_name='standalone-test'\nstate_dir='{root/label}'\nlog_level='info'\n[transport]\nbind_addr='127.0.0.1:0'\nrelay_url='https://127.0.0.1:{port}'\nrelay_only=true\nrelay_ca_file='{certificate}'\n")
                return path
            host=node_config('host',2);client=node_config('client',1)
            pin=run([BINARY,'host-init','--config',host]).stdout.split('Endpoint ID: ')[1].strip()
            peer=run([BINARY,'identity-init','--config',client]).stdout.split('Endpoint ID: ')[1].strip()
            request=root/'request.json';request.write_text(json.dumps({'op':'approve','expected_revision':0,'peer':peer,'name':'test'}))
            run([BINARY,'host-mutate','--config',host,'--request',request])
            agent=subprocess.Popen([BINARY,'agent','--config',host],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            try:
                event(agent,agent.stderr,'started')
                catalog=json.loads(run([BINARY,'host-catalog','--config',client,'--peer',pin]).stdout)
                assert catalog['host']==pin and catalog['services']==[]
            finally: stop(agent)

        finally: stop(relay)
        # Secret protection errors are fixed labels and must not dump PEM bytes.
        key.chmod(0o644)
        rejected=run(command,ok=False)
        assert 'protected' in rejected.stderr and 'PRIVATE KEY' not in rejected.stderr
    print('Standalone relay: verified TLS, wrong hostname/key-permission rejection, no relay-owned node state, native Iroh catalog and graceful shutdown passed')

if __name__=='__main__':main()
