#!/usr/bin/env python3
"""Physical app lifecycle test. Temporarily cycles tablet Wi-Fi and relay service.

Requires existing approved app pairings and installed production/test debug APKs.
A unique host-local fixture grant is removed on exit. No ADB reverse is used.
"""
import argparse
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import re
from pathlib import Path
import shlex
import subprocess
import tempfile
import threading
import time
import uuid
from relay_smoke import run
from physical_relay_smoke import SSH


class Fixture(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *args):
        pass

    def reply(self, text):
        body=text.encode();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)

    def do_GET(self):
        if self.path=='/ws':
            digest=base64.b64encode(hashlib.sha1((self.headers['Sec-WebSocket-Key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
            self.send_response(101);self.send_header('Upgrade','websocket');self.send_header('Connection','Upgrade');self.send_header('Sec-WebSocket-Accept',digest);self.end_headers()
            first=self.rfile.read(2)
            if len(first)!=2:return
            size=first[1]&127;mask=self.rfile.read(4);body=self.rfile.read(size)
            if first[0]!=0x81 or not first[1]&128 or size>125 or len(mask)!=4:return
            body=bytes(b^mask[i%4] for i,b in enumerate(body));self.wfile.write(bytes([0x81,len(body)])+body);self.wfile.flush();self.close_connection=True
        elif self.path=='/stream':
            self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Connection','close');self.end_headers();self.close_connection=True
            try:
                for i in range(40):self.wfile.write(f'data: {i}\n\n'.encode());self.wfile.flush();time.sleep(1)
            except (BrokenPipeError,ConnectionResetError):pass
        elif self.path=='/count':
            self.reply(f'post-count={self.server.posts}')
        else:self.reply('lifecycle-fixture')

    def do_POST(self):
        with self.server.count_lock:self.server.posts+=1
        self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Connection','close');self.end_headers();self.close_connection=True
        try:
            self.wfile.write(b'data: accepted\n\n');self.wfile.flush()
            while not self.server.stopping.wait(1):self.wfile.write(b': waiting\n\n');self.wfile.flush()
        except (BrokenPipeError,ConnectionResetError):pass


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial',required=True)
    parser.add_argument('--peer',help='public tablet Endpoint ID if multiple Android clients are approved')
    parser.add_argument('--force-relay',action='store_true',help='temporarily disable direct IP transport on both host agents')
    parser.add_argument('--wait-for-network-switch',action='store_true',help='wait for /tmp/wispnet-network-switch-ready before continuing')
    args=parser.parse_args();hosts=json.loads(Path('/tmp/wispnet-app-hosts.json').read_text());host=hosts[0]
    def adb(*cmd,**kw):return run(['adb','-s',args.serial,*cmd],**kw)
    def owner(command,*tail):return run([host['binary'],command,'--config',host['config'],*tail])
    def host_command(record,cmd,**kwargs):
        return run([*SSH,'desktop.home',shlex.join(cmd)],**kwargs) if record['remote'] else run(cmd,**kwargs)
    def write_config(record,content):
        host_command(record,['python3','-c',"import os,sys;f=os.open(sys.argv[1],os.O_WRONLY|os.O_TRUNC|os.O_NOFOLLOW);os.write(f,sys.stdin.buffer.read());os.close(f)",record['config']],input=content)
    original_configs=[]

    def relay(action):return run([*SSH,'desktop.home','systemctl --user '+action+' wipsnet-relay.service'])
    def ack():adb('shell','run-as','app.wispnet.android','touch','files/lifecycle-continue')
    def awake():adb('shell','input','keyevent','224');adb('shell','wm','dismiss-keyguard');adb('shell','input','swipe','600','1400','600','300','300')
    marker=Path('/tmp/wispnet-network-switch-ready');marker.unlink(missing_ok=True)
    fixture='lifecycle-'+uuid.uuid4().hex[:12];registered=False;process=None
    server=ThreadingHTTPServer(('127.0.0.1',0),Fixture);server.daemon_threads=True;server.posts=0;server.count_lock=threading.Lock();server.stopping=threading.Event()
    threading.Thread(target=server.serve_forever,daemon=True).start()
    # Read only the public peer identity recorded by the previous physical test.
    records=json.loads(owner('host-status').stdout)
    clients=records['clients'];matches=[c['peer'] for c in clients if c['name']=='Wispnet Android']
    if args.peer:
        assert args.peer in [c['peer'] for c in clients],'Specified tablet must already be approved'
        tablet=args.peer
    else:
        assert len(matches)==1,'Use --peer to choose the intended approved tablet identity'
        tablet=matches[0]
    old_timeout=adb('shell','settings','get','system','screen_off_timeout').stdout.strip()
    old_stayon=adb('shell','settings','get','global','stay_on_while_plugged_in').stdout.strip()
    wifi_initial=adb('shell','cmd','wifi','status').stdout
    assert 'enabled' in wifi_initial,'Connect tablet Wi-Fi before running the lifecycle check'
    print('Initial network: '+wifi_initial.splitlines()[0],flush=True)
    with tempfile.TemporaryDirectory(prefix='wispnet-app-lifecycle-') as temporary:
        request=Path(temporary)/'request.json'
        def mutate(value):
            value['expected_revision']=json.loads(owner('host-status').stdout)['revision'];request.write_text(json.dumps(value));request.chmod(0o600);owner('host-mutate','--request',request)
        def command(mode):return ['adb','-s',args.serial,'shell','am','instrument','-w','-e','host_a',hosts[0]['host'],'-e','host_b',hosts[1]['host'],'-e','fixture',fixture,'-e','mode',mode,'app.wispnet.android.test/com.biplabs.wispnet.AppTestRunner']
        try:
            if args.force_relay:
                for record in hosts:
                    content=host_command(record,['cat',record['config']]).stdout
                    updated,count=re.subn(r'(?m)^relay_only\s*=\s*false\s*$', 'relay_only=true',content)
                    assert count==1,'Expected explicit normal transport config'
                    original_configs.append((record,content));write_config(record,updated)
                    host_command(record,['systemctl','--user','restart','wipsnet-host.service'])
                print('Both host agents use relay-only transports; direct IP transports disabled',flush=True)
            mutate({'op':'service','id':fixture,'backend':f'127.0.0.1:{server.server_port}','allow':[tablet]});registered=True
            adb('shell','am','force-stop','app.wispnet.android');awake();adb('shell','settings','put','system','screen_off_timeout','1800000')
            adb('shell','svc','power','stayon','usb')
            log=Path('/tmp/wispnet-app-lifecycle.log');handled=set();start=time.monotonic()
            with log.open('w') as output:
                process=subprocess.Popen(command('lifecycle'),stdout=output,stderr=output,text=True)
                while True:
                    text=log.read_text()
                    assert time.monotonic()-start<600,'Lifecycle test timed out'
                    if 'result=failed' in text:raise AssertionError(text)
                    stages=[line.split('stage=',1)[1].strip() for line in text.splitlines() if 'stage=' in line]
                    for stage in stages:
                        if stage in handled:continue
                        handled.add(stage);print(f'{stage} ({time.monotonic()-start:.1f}s)',flush=True)
                        if stage=='screen-off-ready':
                            adb('shell','svc','power','stayon','false');adb('shell','input','keyevent','223');time.sleep(1);power=adb('shell','dumpsys','power').stdout;assert 'mWakefulness=Asleep' in power or 'mWakefulness=Dozing' in power,'Screen did not turn off';print('Screen-off state verified',flush=True);time.sleep(29);awake();ack()
                        elif stage=='doze-ready':
                            adb('shell','dumpsys','battery','unplug');adb('shell','input','keyevent','223');idle=adb('shell','dumpsys','deviceidle','force-idle').stdout
                            assert 'idle' in idle.lower(),idle
                            print('Forced device idle: '+adb('shell','dumpsys','deviceidle','get','deep').stdout.strip(),flush=True)
                            time.sleep(15);adb('shell','dumpsys','deviceidle','unforce');adb('shell','dumpsys','battery','reset');awake();ack()
                        elif stage=='wifi-drop-ready':
                            adb('shell','svc','wifi','disable');time.sleep(5);adb('shell','svc','wifi','enable');time.sleep(3);ack()
                        elif stage=='relay-restart-ready':
                            relay('stop');time.sleep(5);relay('start');ack()
                        elif stage=='network-switch-ready':
                            if args.wait_for_network_switch:
                                print('WAITING_FOR_NETWORK_SWITCH: reconnect tablet to home Wi-Fi, then create /tmp/wispnet-network-switch-ready',flush=True)
                                deadline=time.monotonic()+110
                                while not marker.exists():
                                    assert time.monotonic()<deadline,'Network switch confirmation timed out'
                                    time.sleep(.5)
                            ack()
                        elif stage=='kill-ready':
                            adb('shell','am','force-stop','app.wispnet.android');process.wait(timeout=15)
                            cold=run(command('lifecycle-cold'),timeout=120);log.write_text(text+'\n'+cold.stdout)
                            assert 'result=passed' in cold.stdout,cold.stdout
                            assert server.posts==1,'Side-effecting POST was replayed'
                            print('Process kill, cold restart, both pairings and explicit restart passed; POST count=1',flush=True)
                            return
                    assert process.poll() is None,'Instrumentation exited before all lifecycle stages: '+text
                    time.sleep(.2)
        finally:
            # Attempt every restoration even if one device/SSH operation fails.
            cleanup_errors=[]
            def cleanup(action):
                try:action()
                except Exception as failure:cleanup_errors.append(failure)
            if process and process.poll() is None:
                cleanup(lambda:adb('shell','am','force-stop','app.wispnet.android',ok=None))
                cleanup(lambda:process.wait(timeout=15))
            cleanup(lambda:adb('shell','am','force-stop','app.wispnet.android',ok=None))
            cleanup(lambda:adb('shell','svc','wifi','enable',ok=None))
            cleanup(lambda:adb('shell','dumpsys','deviceidle','unforce',ok=None))
            cleanup(lambda:adb('shell','dumpsys','battery','reset',ok=None))
            cleanup(lambda:adb('shell','settings','put','system','screen_off_timeout',old_timeout,ok=None))
            cleanup(lambda:adb('shell','settings','put','global','stay_on_while_plugged_in',old_stayon,ok=None))
            cleanup(lambda:relay('start'));server.stopping.set();server.shutdown();server.server_close()
            if registered:cleanup(lambda:mutate({'op':'remove','id':fixture}))
            for record,content in reversed(original_configs):
                cleanup(lambda record=record,content=content:write_config(record,content))
                cleanup(lambda record=record:host_command(record,['systemctl','--user','restart','wipsnet-host.service']))
            cleanup(lambda:adb('shell','run-as','app.wispnet.android','rm','-f','files/lifecycle-continue','files/lifecycle-evidence.json',ok=None))
            marker.unlink(missing_ok=True);cleanup(awake)
            if cleanup_errors:raise RuntimeError('Lifecycle cleanup failed; inspect device and host state') from cleanup_errors[0]

if __name__=='__main__':main()
