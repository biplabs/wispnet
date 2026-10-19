#!/usr/bin/env python3
"""Compare native HTTPS egress and verify the public relay from an ADB tablet.

Requires a current cross-compiled external_network_probe example. Only its binary
is copied; no relay/node credentials. Public IPs are compared, not printed.
"""
import argparse
import json
from pathlib import Path
import shlex
import shutil
import tempfile
import uuid
from relay_smoke import ROOT, run


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial',required=True)
    args=parser.parse_args()
    local=json.loads(run([ROOT/'target/debug/examples/external_network_probe'],timeout=20).stdout)
    remote='/data/local/tmp/wispnet-external-preflight-'+uuid.uuid4().hex
    def adb(*command):return run(['adb','-s',args.serial,*command],timeout=25)
    def tablet(*command):return adb('shell','-T',shlex.join(command))
    created=False
    try:
        tablet('mkdir','-m','700',remote);created=True
        with tempfile.TemporaryDirectory(prefix='wispnet-external-probe-') as temporary:
            binary=Path(temporary)/'probe';shutil.copyfile(ROOT/'target/aarch64-linux-apps/android/debug/examples/external_network_probe',binary)
            run([Path.home()/'Android/Sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip',binary])
            adb('push',binary,remote+'/probe')
        tablet('chmod','700',remote+'/probe')
        external=json.loads(tablet('timeout','20',remote+'/probe').stdout)
        assert local['relay_https'] and external['relay_https']
        different=local['public_ip']!=external['public_ip']
        print(json.dumps({'event':'external_network_preflight','different_egress':different,'both_verified_public_relay_https':True}))
        assert different,'tablet and home computer share the same observed public egress; not an external-network test'
    finally:
        if created:tablet('rm','-rf',remote)

if __name__=='__main__':main()
