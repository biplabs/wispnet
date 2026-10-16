#!/usr/bin/env python3
"""Install only the standalone temporary relay in a trusted SSH user's account.

TLS key is generated on the relay and never copied out. This does not install,
initialize, restart or modify a Wispnet node or existing web/relay services.
"""
import argparse
import ipaddress
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
from physical_relay_smoke import SSH
from relay_smoke import ROOT, run


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ssh-host',required=True)
    parser.add_argument('--hostname',required=True)
    parser.add_argument('--lan-ip',required=True)
    args=parser.parse_args()
    assert args.ssh_host and not args.ssh_host.startswith('-') and all(c.isalnum() or c in '.-_@' for c in args.ssh_host)
    assert all(c.isalnum() or c in '.-' for c in args.hostname) and '.' in args.hostname and not args.hostname.startswith('-')
    ipaddress.IPv4Address(args.lan_ip)
    def remote(*arguments,**kwargs): return run([*SSH,args.ssh_host,shlex.join([str(a) for a in arguments])],**kwargs)
    subprocess.run(['cargo','build','-p','wipsnet-relay','--locked'],cwd=ROOT,check=True)
    home=remote('python3','-c','from pathlib import Path;print(Path.home())').stdout.strip()
    assert home.startswith('/') and '\n' not in home
    base=home+'/.local/libexec/wipsnet-relay'
    config=home+'/.config/wipsnet-relay'
    tls=home+'/.local/share/wipsnet-relay'
    # Existing protected directories must retain ownership/mode and cannot be symlinks.
    remote('python3','-c',"""import os,pathlib,sys
for item in sys.argv[1:]:
 p=pathlib.Path(item)
 if p.exists() or p.is_symlink():
  m=p.lstat();assert p.is_dir() and not p.is_symlink() and m.st_uid==os.getuid() and m.st_mode&0o077==0,'unsafe relay directory'
 else: p.mkdir(mode=0o700,parents=True)
""",base,config,tls)
    certificate=tls+'/relay-cert.pem';key=tls+'/relay-key.pem'
    ca=tls+'/relay-ca.pem';ca_key=tls+'/relay-ca-key.pem'
    present=remote('test','-f',key,ok=None).returncode==0
    if not present:
        # All secret keys are generated on desktop. The CA is distinct from
        # the server leaf: WebPKI correctly rejects CA certificates as leaves.
        remote('python3','-c',"""import os,pathlib,secrets,subprocess,sys,tempfile
os.umask(0o077)
key,cert,ca_key,ca,hostname,ip=sys.argv[1:]
def openssl(*args): subprocess.run(['openssl',*args],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
if not pathlib.Path(ca_key).exists():
 assert not pathlib.Path(ca).exists()
 openssl('req','-x509','-newkey','ed25519','-nodes','-days','365','-keyout',ca_key,'-out',ca,
  '-subj','/CN=Wipsnet temporary relay CA','-addext','basicConstraints=critical,CA:TRUE',
  '-addext','keyUsage=critical,keyCertSign,cRLSign')
with tempfile.TemporaryDirectory(dir=pathlib.Path(key).parent) as tmp:
 csr=tmp+'/relay.csr';ext=tmp+'/extensions'
 openssl('req','-new','-newkey','ed25519','-nodes','-keyout',key,'-out',csr,'-subj','/CN='+hostname)
 pathlib.Path(ext).write_text('basicConstraints=critical,CA:FALSE\\nkeyUsage=critical,digitalSignature\\nextendedKeyUsage=serverAuth\\nsubjectAltName=DNS:'+hostname+',DNS:desktop.home,IP:'+ip+'\\n')
 openssl('x509','-req','-in',csr,'-CA',ca,'-CAkey',ca_key,'-set_serial',str(secrets.randbits(128)),
  '-days','30','-out',cert,'-extfile',ext)
""",key,certificate,ca_key,ca,args.hostname,args.lan_ip)
    remote('openssl','x509','-in',certificate,'-noout','-checkhost',args.hostname)
    remote('openssl','x509','-in',certificate,'-noout','-checkend','86400')
    with tempfile.TemporaryDirectory(prefix='wipsnet-relay-install-') as temporary:
        root=Path(temporary);binary=root/'wipsnet-relay'
        shutil.copyfile(ROOT/'target/debug/wipsnet-relay',binary);run(['strip','--strip-debug',binary])
        run(['scp','-q','-o','BatchMode=yes','-o','StrictHostKeyChecking=yes',binary,args.ssh_host+':'+base+'/wipsnet-relay.next'])
    remote('python3','-c',"""import os,sys
p=sys.argv[1];os.chmod(p+'.next',0o700);os.replace(p+'.next',p)
""",base+'/wipsnet-relay')
    text=f"listen='0.0.0.0:8443'\ncertificate='{certificate}'\nprivate_key='{key}'\nmax_connections=64\n"
    remote('python3','-c',"""import os,sys
p=sys.argv[1];f=os.open(p+'.next',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(f,'w') as out: out.write(sys.stdin.read())
os.replace(p+'.next',p)
""",config+'/relay.toml',input=text)
    unit=remote('python3','-c',"from pathlib import Path;print(Path.home()/'.config/systemd/user/wipsnet-relay.service')").stdout.strip()
    remote('python3','-c',"""import os,sys
p=sys.argv[1];f=os.open(p+'.next',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(f,'w') as out: out.write(sys.stdin.read())
os.replace(p+'.next',p)
""",unit,input=(ROOT/'deploy/relay/wipsnet-relay.service').read_text())
    remote('systemctl','--user','daemon-reload')
    remote('systemctl','--user','enable','wipsnet-relay.service')
    remote('systemctl','--user','restart','wipsnet-relay.service')
    remote('systemctl','--user','is-active','wipsnet-relay.service')
    remote('curl','--silent','--show-error','--fail','--cacert',ca,f'https://{args.lan_ip}:8443/ping')
    print('Standalone relay installed: TCP 8443; remote TLS key retained; no node state touched',flush=True)
    print('Public CA certificate: '+args.ssh_host+':'+ca,flush=True)

if __name__=='__main__':main()
