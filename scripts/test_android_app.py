#!/usr/bin/env python3
"""Drive app pairing UI, host-local approvals/grants and ordinary Chrome handoff.

Requires installed debug app/test APKs and persistent hosts prepared by
install_app_hosts.py. Does not copy private host/tablet keys or use TCP forwards.
"""
import argparse
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import uuid
from physical_relay_smoke import SSH
from relay_smoke import ROOT,run


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--serial',required=True);parser.add_argument('--resume',action='store_true');args=parser.parse_args()
    hosts=json.loads(Path('/tmp/wispnet-app-hosts.json').read_text())
    def adb(*command,**kwargs):return run(['adb','-s',args.serial,*command],**kwargs)
    def host_call(host,*command,**kwargs):
        cmd=[host['binary'],*command,'--config',host['config']]
        # CLI requires mode/config first, then command-specific arguments.
        cmd=[host['binary'],command[0],'--config',host['config'],*command[1:]]
        return run([*SSH,'desktop.home',shlex.join(cmd)],**kwargs) if host['remote'] else run(cmd,**kwargs)
    def write_owner_request(host,path,value):
        text=json.dumps(value)
        if host['remote']:
            run([*SSH,'desktop.home',shlex.join(['python3','-c','import os,sys;f=os.open(sys.argv[1],os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600);os.write(f,sys.stdin.buffer.read());os.close(f)',str(path)])],input=text)
        else:Path(path).write_text(text)
    invitations=[];process=None;remote_files=[]
    with tempfile.TemporaryDirectory(prefix='wispnet-app-test-') as temporary:
        root=Path(temporary)
        try:
            if not args.resume:
                for i,host in enumerate(hosts):
                    path=host['state']+'/app-invitation-'+uuid.uuid4().hex+'.json' if host['remote'] else str(root/f'invitation-{i}.json')
                    issued=json.loads(host_call(host,'host-invite','--output',path).stdout);invitations.append(issued['invitation'])
                    if host['remote']:
                        remote_files.append(path);local=root/f'invitation-{i}.json'
                        run(['scp','-q','-o','BatchMode=yes','-o','StrictHostKeyChecking=yes','desktop.home:'+path,local])
                    else:local=Path(path)
                    adb('shell','-T',shlex.join(['run-as','com.biplabs.wispnet','sh','-c',f'umask 077; cat > files/invitation-{i}.json']),input=local.read_text())
            adb('shell','pm','grant','com.biplabs.wispnet','android.permission.POST_NOTIFICATIONS')
            command=['adb','-s',args.serial,'shell','am','instrument','-w','-e','host_a',hosts[0]['host'],'-e','host_b',hosts[1]['host']]
            if args.resume:command+=['-e','mode','resume']
            command+=['com.biplabs.wispnet.test/com.biplabs.wispnet.AppTestRunner']
            with open('/tmp/wispnet-android-instrumentation.log','w') as log:
                process=subprocess.Popen(command,stdout=log,stderr=log,text=True)
                if not args.resume:
                    for i,host in enumerate(hosts):
                        deadline=time.monotonic()+60
                        while True:
                            entries=json.loads(host_call(host,'host-invitations').stdout)
                            entry=next((e for e in entries if e['invitation']==invitations[i] and e['state']=='pending'),None)
                            if entry:break
                            assert process.poll() is None and time.monotonic()<deadline,'app pairing did not reach pending state; see instrumentation log'
                            time.sleep(.2)
                        status=json.loads(host_call(host,'host-status').stdout)
                        request_path=host['state']+'/app-test-request.json' if host['remote'] else root/'request.json'
                        if host['remote']:remote_files.append(str(request_path))
                        value={'op':'approve_invitation','expected_revision':status['revision'],'invitation':invitations[i]}
                        write_owner_request(host,request_path,value);host_call(host,'host-mutate','--request',str(request_path))
                        services=[('website','127.0.0.1:8080')] if host['remote'] else [('website','127.0.0.1:8310'),('llama','127.0.0.1:8201')]
                        for service,backend in services:
                            revision=json.loads(host_call(host,'host-status').stdout)['revision']
                            value={'op':'service','expected_revision':revision,'id':service,'backend':backend,'allow':[entry['peer']]}
                            write_owner_request(host,request_path,value);host_call(host,'host-mutate','--request',str(request_path))
                        print(f"{host['name']}: app invitation approved locally; intended services granted",flush=True)
                process.wait(timeout=240)
            output=Path('/tmp/wispnet-android-instrumentation.log').read_text()
            print(output)
            assert 'result=passed' in output and process.returncode==0,'Android app UI/browser test failed'
        finally:
            if process and process.poll() is None:process.terminate();process.wait(timeout=10)
            for file in remote_files:run([*SSH,'desktop.home',shlex.join(['rm','-f',file])])
            for i in range(2):adb('shell','run-as','com.biplabs.wispnet','rm','-f',f'files/invitation-{i}.json')
    print('Installed app pairing, independent catalogs and normal-browser handoff passed; hosts/app pairing retained')

if __name__=='__main__':main()
