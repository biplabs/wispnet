#!/usr/bin/env python3
"""Private Sites wire test on two Linux computers and an ADB tablet.

Reads the existing Sites service token using hidden terminal input. Only probe
binaries are copied; no token/key files, USB reverse or SSH forwards are used.
This tests Iroh wire authentication and datagrams, not the application or QUIC.
"""
import argparse
import getpass
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import uuid
from physical_relay_smoke import SSH
from relay_smoke import ROOT, run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ssh-host', required=True)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--url', required=True)
    parser.add_argument('--durable', action='store_true', help='require the Durable Object route; no isolate fallback')
    args = parser.parse_args()
    assert args.url.startswith('https://') and args.url.endswith('.chatgpt.site')
    assert all(c.isalnum() or c in '.-_@' for c in args.ssh_host) and not args.ssh_host.startswith('-')
    token = getpass.getpass('Existing Sites service token (hidden): ')
    assert token and '\n' not in token and '\r' not in token
    url = 'wss://' + args.url.removeprefix('https://') + ('/relay-durable' if args.durable else '/relay')
    suffix = uuid.uuid4().hex
    remote = '/tmp/wispnet-sites-' + suffix
    device = '/data/local/tmp/wispnet-sites-' + suffix
    binary = ROOT / 'target/debug/examples/sites_relay_probe'
    android = ROOT / 'target/aarch64-linux-apps/android/debug/examples/sites_relay_probe'
    assert binary.is_file() and android.is_file()
    def ssh(*command):
        return [*SSH, args.ssh_host, shlex.join(command)]
    def adb(*command):
        return ['adb','-s',args.serial,*command]
    def tablet(*command):
        return adb('shell','-T',shlex.join(command))
    created_remote = created_tablet = False
    processes = []
    results = []
    try:
        run(ssh('mkdir','-m','700',remote));created_remote=True
        run(tablet('mkdir','-m','700',device));created_tablet=True
        with tempfile.TemporaryDirectory(prefix='wispnet-sites-copy-') as tmp:
            import shutil
            native = Path(tmp)/'probe';shutil.copyfile(binary,native)
            run(['strip','--strip-debug',native])
            arm = Path(tmp)/'apps/android';shutil.copyfile(android,arm)
            run([Path.home()/'Android/Sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip',arm])
            run(['scp','-q','-o','BatchMode=yes','-o','StrictHostKeyChecking=yes',native,args.ssh_host+':'+remote+'/probe'])
            run(adb('push',arm,device+'/probe'))
        run(ssh('chmod','700',remote+'/probe'));run(tablet('chmod','700',device+'/probe'))
        def launch(label,mode,destination=None):
            command = [str(binary),url,mode]
            if label=='desktop': command=ssh('timeout','40',remote+'/probe',url,mode)
            if label=='tablet': command=tablet('timeout','40',device+'/probe',url,mode)
            p=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            processes.append(p)
            p.stdin.write(token+'\n'+(destination+'\n' if destination else ''));p.stdin.flush()
            line=p.stdout.readline()
            try: event=json.loads(line)
            except ValueError: event={'event':'no_authentication'}
            return p,event
        for listener,sender in [('local','desktop'),('desktop','tablet'),('tablet','local')]:
            lp,first=launch(listener,'listen')
            print(json.dumps({'listener':listener,'event':first['event'],'status':first.get('status')}),flush=True)
            if first['event']!='authenticated':
                lp.communicate(timeout=40);results.append(False);continue
            sp,second=launch(sender,'send',first['peer'])
            print(json.dumps({'sender':sender,'event':second['event'],'status':second.get('status')}),flush=True)
            sout,serr=sp.communicate(timeout=40)
            lout,lerr=lp.communicate(timeout=40)
            success=sp.returncode==0 and lp.returncode==0 and 'packet_roundtrip' in sout and 'packet_roundtrip' in lout
            # Never dump error text or request headers containing access credentials.
            print(json.dumps({'pair':listener+'-'+sender,'roundtrip':success,'sender_events':sout.strip(),'listener_events':lout.strip()}),flush=True)
            results.append(success)
    finally:
        for p in processes:
            if p.poll() is None: p.terminate()
            try: p.communicate(timeout=5)
            except subprocess.TimeoutExpired: p.kill();p.communicate()
        if created_tablet: run(tablet('rm','-rf',device))
        if created_remote: run(ssh('rm','-rf',remote))
    print(json.dumps({'all_pairs_passed':all(results),'pairs_tested':len(results),'temporary_state_removed':True}),flush=True)
    return 0 if len(results)==3 and all(results) else 1

if __name__=='__main__':
    raise SystemExit(main())
