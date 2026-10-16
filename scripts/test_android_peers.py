#!/usr/bin/env python3
"""Fresh independent pairing and revocation on a separate physical test app.

Stops one node role at a time, never the relay. Leaves the installed user's app,
its identity, approvals and real service grants untouched. Test peers/fixtures
are revoked/removed afterward; both original node services are restarted.
"""
import json
from pathlib import Path
import select
import shlex
import subprocess
import tempfile
import time
import uuid
import argparse
from relay_smoke import ROOT,run
from physical_relay_smoke import SSH
PKG='com.biplabs.wispnet.peercheck'

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--serial',required=True);parser.add_argument('--order',choices=['both','local-first','desktop-first'],default='both');args=parser.parse_args()
    hosts=json.loads(Path('/tmp/wispnet-app-hosts.json').read_text());tag=uuid.uuid4().hex[:12];fixture='peer-'+tag
    processes=[];test_peers=set();remote_files=[];registered=[];current_peer=[None];issued=[]
    def adb(*cmd,**kw):return run(['adb','-s',args.serial,*cmd],**kw)
    def command(h,cmd,**kw):return run([*SSH,'desktop.home',shlex.join(cmd)],**kw) if h['remote'] else run(cmd,**kw)
    def cli(h,op,*tail):return command(h,[h['binary'],op,'--config',h['config'],*map(str,tail)])
    def node(h,action):command(h,['systemctl','--user',action,'wipsnet-host.service'])
    def status(h):return json.loads(cli(h,'host-status').stdout)
    def write(h,path,text):command(h,['python3','-c','import os,sys;f=os.open(sys.argv[1],os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600);os.write(f,sys.stdin.buffer.read());os.close(f)',str(path)],input=text)
    with tempfile.TemporaryDirectory(prefix='wispnet-peer-check-') as temp:
        root=Path(temp)
        def mutate(h,value):
            value['expected_revision']=status(h)['revision'];path=h['state']+'/peer-request-'+tag+'.json' if h['remote'] else str(root/'request.json')
            if h['remote']:remote_files.append(path)
            write(h,path,json.dumps(value));cli(h,'host-mutate','--request',path)
        def grant(h,peer):mutate(h,{'op':'service','id':fixture,'backend':f"127.0.0.1:{h['fixture_port']}",'allow':[peer]})
        def instrument(mode,target,other,on_stage=None):
            print(mode+': '+target['name'],flush=True)
            adb('shell','am','force-stop',PKG)
            cmd=['am','instrument','-w','-e','mode',mode,'-e','host_a',target['host'],'-e','host_b',other['host'],'-e','name',target['name'],'-e','fixture',fixture,PKG+'.test/com.biplabs.wispnet.AppTestRunner']
            path=root/(mode+'-'+target['name'].replace(' ','')+'.log')
            with path.open('w') as output:
                p=subprocess.Popen(['adb','-s',args.serial,'shell',shlex.join(cmd)],stdout=output,stderr=output,text=True);processes.append(p);seen=set();deadline=time.monotonic()+150
                while p.poll() is None:
                    assert time.monotonic()<deadline,'Peer test timed out'
                    for line in path.read_text().splitlines():
                        if 'stage=' in line:
                            stage=line.split('stage=',1)[1].strip()
                            if stage not in seen:
                                seen.add(stage);print(mode+': '+stage,flush=True)
                                if on_stage:on_stage(stage)
                    time.sleep(.15)
            text=path.read_text();assert 'result=passed' in text,text
        try:
            for h in hosts:
                if h['remote']:
                    path=h['state']+'/peer-fixture-'+tag+'.py';remote_files.append(path);write(h,path,(ROOT/'scripts/peer_fixture.py').read_text());cmd=[*SSH,'desktop.home',shlex.join(['python3','-u',path,h['name']])]
                else:cmd=['python3','-u',str(ROOT/'scripts/peer_fixture.py'),h['name']]
                p=subprocess.Popen(cmd,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True);processes.append(p)
                assert select.select([p.stdout],[],[],10)[0],'Fixture startup timeout';h['fixture_port']=json.loads(p.stdout.readline())['port']
            # Each ordering starts with a fresh, separately sandboxed Android identity.
            orders=[(1,0),(0,1)] if args.order=='both' else ([(0,1)] if args.order=='local-first' else [(1,0)])
            for order in orders:
                adb('shell','am','force-stop',PKG);adb('shell','pm','clear',PKG);adb('shell','run-as',PKG,'mkdir','-p','files');adb('shell','pm','grant',PKG,'android.permission.POST_NOTIFICATIONS')
                for index in order:
                    h=hosts[index];other=hosts[1-index];node(h,'start');node(other,'stop')
                    path=h['state']+'/peer-invitation-'+tag+'.json' if h['remote'] else str(root/'invite.json')
                    if h['remote']:remote_files.append(path)
                    invitation=json.loads(cli(h,'host-invite','--output',path).stdout)['invitation'];issued.append((h,invitation))
                    content=command(h,['cat',path]).stdout
                    adb('shell','-T',shlex.join(['run-as',PKG,'sh','-c','umask 077; cat > files/peer-invitation.json']),input=content)
                    def approve(stage):
                        if stage!='pending-owner-approval':return
                        entry=next(e for e in json.loads(cli(h,'host-invitations').stdout) if e['invitation']==invitation and e['state']=='pending');peer=entry['peer'];test_peers.add(peer);current_peer[0]=peer
                        mutate(h,{'op':'approve_invitation','invitation':invitation});grant(h,peer)
                        if h not in registered:registered.append(h)
                        adb('shell','run-as',PKG,'touch','files/peers-continue')
                    instrument('peers-pair',h,other,approve)
                    instrument('peers-cold',h,other)
                    command(h,['rm','-f',path])
                for h in hosts:node(h,'start')
            assert len(test_peers)==len(orders),'Clearing the separate test app must produce distinct identities'
            # Last ordering leaves a single test identity paired with both hosts.
            def service_grant(stage):
                if stage=='withdraw-service-grant':mutate(hosts[0],{'op':'service','id':fixture,'backend':f"127.0.0.1:{hosts[0]['fixture_port']}",'allow':[]})
                elif stage=='restore-service-grant':grant(hosts[0],current_peer[0])
                else:return
                adb('shell','run-as',PKG,'touch','files/peers-continue')
            instrument('peers-service-deny',hosts[0],hosts[1],service_grant)
            def revoke(stage):
                h=hosts[0]
                if stage=='revoke-active-host':
                    peer=current_peer[0]
                    mutate(h,{'op':'revoke','peer':peer});adb('shell','run-as',PKG,'touch','files/peers-continue')
                elif stage=='restart-revoked-host':node(h,'restart');adb('shell','run-as',PKG,'touch','files/peers-continue')
            instrument('peers-revoke',hosts[0],hosts[1],revoke)
            instrument('peers-denied-cold',hosts[0],hosts[1])
            print('PASS: '+args.order+' pairing order; cold independent browsers, pin/replay rejection, active revocation and host isolation',flush=True)
        finally:
            adb('shell','am','force-stop',PKG,ok=None)
            errors=[]
            for h in hosts:
                try:
                    node(h,'start')
                    if any(s['id']==fixture for s in status(h)['services']):mutate(h,{'op':'remove','id':fixture})
                    for c in status(h)['clients']:
                        if c['peer'] in test_peers:mutate(h,{'op':'revoke','peer':c['peer']})
                except Exception as failure:errors.append(failure)
            for h,invitation in issued:
                try:
                    entries=json.loads(cli(h,'host-invitations').stdout)
                    if any(e['invitation']==invitation and e['state'] in ('issued','challenged','pending') for e in entries):mutate(h,{'op':'reject_invitation','invitation':invitation})
                except Exception as failure:errors.append(failure)
            # Closing SSH alone can orphan a remote HTTP server. Terminate only
            # this exact fixture command owned by the remote user before unlinking.
            for path in remote_files:
                if path.endswith('.py'):
                    code='import os,sys,signal,pathlib; target=sys.argv[1]; '+ '\n' + 'for p in pathlib.Path("/proc").iterdir():\n try:\n  a=(p/"cmdline").read_bytes().split(bytes([0])); matched=p.stat().st_uid==os.getuid() and len(a)>3 and a[1]==b"-u" and a[2].decode()==target\n  if matched: os.kill(int(p.name),signal.SIGTERM)\n except (OSError,ValueError): pass'
                    try:command(hosts[1],['python3','-c',code,path])
                    except Exception as failure:errors.append(failure)
            for p in processes:
                if p.poll() is None:p.terminate()
                try:p.wait(timeout=10)
                except subprocess.TimeoutExpired:p.kill();p.wait()
            for path in set(remote_files):command(hosts[1],['rm','-f',path])
            adb('uninstall',PKG,ok=None);adb('uninstall',PKG+'.test',ok=None)
            if errors:raise RuntimeError('Peer test cleanup failed') from errors[0]

if __name__=='__main__':main()
