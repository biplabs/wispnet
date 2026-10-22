#!/usr/bin/env python3
"""Install independent local/desktop host nodes for the Android app; no grants copied."""
from pathlib import Path
import json
import os
import shlex
import shutil
import subprocess
import tempfile
from physical_relay_smoke import SSH
from relay_smoke import ROOT,run


def main():
    subprocess.run(['cargo','build','-p','wispnet-cli','--locked'],cwd=ROOT,check=True)
    records=[]
    for remote in [False,True]:
        def call(args,**kwargs):
            return run([*SSH,'desktop.home',shlex.join([str(a) for a in args])],**kwargs) if remote else run(args,**kwargs)
        home=call(['python3','-c','from pathlib import Path;print(Path.home())']).stdout.strip()
        binary=home+'/.local/libexec/wispnet-node/wispnet';config=home+'/.config/wispnet-node/host.toml';state=home+'/.local/state/wispnet-node'
        call(['python3','-c',"""import os,pathlib,sys
for item in sys.argv[1:]:
 p=pathlib.Path(item)
 if not p.exists(): p.mkdir(parents=True,mode=0o700)
 m=p.lstat();assert p.is_dir() and not p.is_symlink() and m.st_uid==os.getuid() and m.st_mode&0o077==0
""",str(Path(binary).parent),str(Path(config).parent),state])
        with tempfile.TemporaryDirectory(prefix='wispnet-node-copy-') as temporary:
            stripped=Path(temporary)/'wispnet';shutil.copyfile(ROOT/'target/debug/wispnet',stripped);run(['strip','--strip-debug',stripped])
            if remote:run(['scp','-q','-o','BatchMode=yes','-o','StrictHostKeyChecking=yes',stripped,'desktop.home:'+binary+'.next'])
            else:shutil.copyfile(stripped,binary+'.next')
        call(['python3','-c','import os,sys;os.chmod(sys.argv[1]+".next",0o700);os.replace(sys.argv[1]+".next",sys.argv[1])',binary])
        content=f"schema_version=2\nstate_dir='{state}'\nlog_level='info'\n[transport]\nbind_addr='0.0.0.0:0'\nrelay_url='https://relay.wispnet.app'\nrelay_only=false\n"
        if call(['test','-f',config],ok=None).returncode:
            call(['python3','-c',"import os,sys; f=os.open(sys.argv[1],os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.write(f,sys.stdin.buffer.read());os.close(f)",config],input=content)
        status=call([binary,'host-status','--config',config],ok=None)
        if status.returncode:
            call([binary,'host-init','--config',config])
            status=call([binary,'host-status','--config',config])
        host=json.loads(status.stdout)['host']
        unit=home+'/.config/systemd/user/wispnet-host.service'
        call(['python3','-c',"import os,sys; p=sys.argv[1]; f=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600);os.write(f,sys.stdin.buffer.read());os.close(f)",unit],input=(ROOT/'deploy/node/wispnet-host.service').read_text())
        call(['systemctl','--user','daemon-reload']);call(['systemctl','--user','enable','wispnet-host.service']);call(['systemctl','--user','restart','wispnet-host.service']);call(['systemctl','--user','is-active','wispnet-host.service'])
        name=call(['python3','-c',"import pathlib,socket,sys,tomllib;print(tomllib.loads(pathlib.Path(sys.argv[1]).read_text()).get('device_name',socket.gethostname()))",config]).stdout.strip()
        records.append({'host':host,'name':name,'remote':remote,'config':config,'binary':binary,'state':state})
    Path('/tmp/wispnet-app-hosts.json').write_text(json.dumps(records))
    print('Independent host nodes ready; no tablet approval or service grants were inferred')

if __name__=='__main__':main()
