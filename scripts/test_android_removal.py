#!/usr/bin/env python3
"""Physical Android removal tests in peercheck; uses temporary Linux authorities only."""
import argparse
import importlib.util
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
ROOT=Path(__file__).resolve().parents[1]
PKG='app.wispnet.android.peercheck'
spec=importlib.util.spec_from_file_location('removal_api',ROOT/'apps/node/desktop/wispnet_desktop/api.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--serial',required=True);args=parser.parse_args()
    def adb(*command,**kwargs):return subprocess.run(['adb','-s',args.serial,*map(str,command)],text=True,capture_output=True,check=True,**kwargs)
    def private(path,text):
        adb('shell','-T',shlex.join(['run-as',PKG,'sh','-c',f'umask 077; cat > {shlex.quote(path)}']),input=text)
    with tempfile.TemporaryDirectory(prefix='wispnet-android-removal-') as temporary:
        root=Path(temporary);processes={};peers={};apis={};configs={}
        def start(name):
            process=subprocess.Popen([ROOT/'target/debug/wispnet','agent','--config',configs[name]],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);processes[name]=process
            deadline=time.monotonic()+10
            while True:
                assert process.poll() is None,'Fixture node exited'
                try:apis[name].request(op='status');return
                except (OSError,ValueError):
                    assert time.monotonic()<deadline,'Fixture node startup timeout';time.sleep(.02)
        def stop(name):
            process=processes.pop(name);process.terminate();process.wait(timeout=10)
        def host(name):
            config=root/(name+'.toml');configs[name]=config
            config.write_text(f'schema_version=2\ndevice_name="Android removal {name}"\nstate_dir="{root/name}"\nlog_level="info"\n[transport]\nbind_addr="127.0.0.1:0"\nrelay_url="https://relay.wispnet.app"\n')
            output=subprocess.check_output([ROOT/'target/debug/wispnet','host-init','--config',config],text=True)
            peers[name]=output.split('Endpoint ID: ')[1].strip();apis[name]=module.NodeAPI(config);start(name)
        def invitation(name):
            path=root/(name+'-invitation.json')
            subprocess.run([ROOT/'target/debug/wispnet','host-invite','--config',configs[name],'--output',path],capture_output=True,check=True)
            text=path.read_text();private('files/removal-'+name+'.json',text);return json.loads(text)['invitation']
        def approve(name,invitation_id,phone):
            status=apis[name].request(op='status')
            entry=next(e for e in status['invitations'] if e['invitation']==invitation_id and e['state']=='pending')
            assert entry['peer']==phone
            apis[name].request(op='mutate',operation={'op':'approve_invitation','expected_revision':status['host']['revision'],'invitation':invitation_id})
        def instrument(mode,action,phone_expected=None):
            command=['adb','-s',args.serial,'shell','am','instrument','-w','-e','mode',mode,'-e','host_a',peers['a'],'-e','host_b',peers['b'],PKG+'.test/com.biplabs.wispnet.AppTestRunner']
            log=root/(mode+'.log');seen=set();phone=phone_expected;stage=None
            with log.open('w') as output:
                process=subprocess.Popen(command,stdout=output,stderr=output,text=True)
                deadline=time.monotonic()+180
                try:
                    while process.poll() is None:
                        assert time.monotonic()<deadline,'Instrumentation timeout'
                        text=log.read_text()
                        # Each status bundle lists peer before stage on Android.
                        for block in text.split('INSTRUMENTATION_STATUS_CODE:'):
                            fields={}
                            for line in block.splitlines():
                                if line.startswith('INSTRUMENTATION_STATUS: ') and '=' in line:
                                    key,value=line[len('INSTRUMENTATION_STATUS: '):].split('=',1);fields[key]=value
                            current=fields.get('stage')
                            if current and current not in seen and fields.get('peer'):
                                seen.add(current);phone=fields.get('peer',phone);assert phone
                                if phone_expected:assert phone==phone_expected,'Test identity changed'
                                print(mode+': '+current+' · '+phone[:8],flush=True)
                                action(current,phone)
                                private('files/removal-continue','continue')
                        time.sleep(.1)
                finally:
                    if process.poll() is None:process.terminate();process.wait(timeout=10)
            result=log.read_text()
            assert process.returncode==0 and 'result=passed' in result,result
            final={line[len('INSTRUMENTATION_RESULT: '):].split('=',1)[0]:line.split('=',1)[1] for line in result.splitlines() if line.startswith('INSTRUMENTATION_RESULT: ') and '=' in line}
            if phone_expected:assert final.get('peer')==phone_expected,'Cold restart changed test identity'
            return phone
        try:
            host('a');host('b')
            adb('shell','am','force-stop',PKG);adb('shell','pm','clear',PKG)
            adb('shell','run-as',PKG,'mkdir','-p','files');adb('shell','pm','grant',PKG,'android.permission.POST_NOTIFICATIONS')
            ia=invitation('a');ib=invitation('b')
            def action(stage,phone):
                if stage=='approve-a':approve('a',ia,phone)
                elif stage=='approve-b':approve('b',ib,phone)
                elif stage=='host-offline':stop('a')
                elif stage=='host-online':start('a')
                elif stage=='delete-a':apis['a'].request(op='unpair',host=phone)
                elif stage=='phone-removed':
                    try:apis['b'].request(op='catalog',host=phone);raise AssertionError('Phone deletion was not learned')
                    except ValueError as error:assert 'Pairing removed' in str(error),str(error)
                    assert not apis['b'].request(op='status')['host']['clients']
            phone=instrument('removal-both',action)
            adb('shell','am','force-stop',PKG)
            instrument('removal-cold',lambda *args:None,phone)
            # An older desktop has only an approved phone, without a reverse host pin.
            adb('shell','am','force-stop',PKG);adb('shell','pm','clear',PKG)
            adb('shell','run-as',PKG,'mkdir','-p','files/client/peer-pins')
            adb('shell','run-as',PKG,'chmod','700','files/client','files/client/peer-pins')
            private('files/client/peer-pins/'+peers['a']+'.json',json.dumps({'version':1,'host':peers['a'],'relay_url':'https://relay.wispnet.app/'}))
            def legacy(stage,phone):
                if stage=='legacy-ready':
                    status=apis['a'].request(op='status')
                    apis['a'].request(op='mutate',operation={'op':'approve','peer':phone,'name':'Legacy phone','expected_revision':status['host']['revision']})
                elif stage=='legacy-phone-removed':
                    deadline=time.monotonic()+45
                    while any(c['peer']==phone for c in apis['a'].request(op='status',refresh_clients=True)['host']['clients']):
                        assert time.monotonic()<deadline,'Legacy client-only removal not learned';time.sleep(.2)
            instrument('removal-legacy',legacy)
            print('Physical Android: both directions, timeout/pending/empty safety, consent cleanup, cold restart and legacy client-only cleanup passed',flush=True)
        finally:
            for name in list(processes):stop(name)
            adb('shell','am','force-stop',PKG)
if __name__=='__main__':main()
