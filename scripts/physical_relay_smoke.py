#!/usr/bin/env python3
"""Two physical Linux authorities plus an authorized Android native client.

Only creates owned private temporary state. The local TLS relay is reached over
an SSH reverse forward and USB reverse; no production relay or Android APK.
Opposite Wipsnet host processes stop; the underlying computers stay powered on.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import tempfile
import time
import uuid
from relay_smoke import BINARY, FIXTURE, ROOT, event, run, stop

SSH = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "StrictHostKeyChecking=yes"]

SUPERVISOR = """import pathlib,signal,subprocess,sys
p=subprocess.Popen(sys.argv[2:],stdin=subprocess.DEVNULL)
def interrupted(*args): raise SystemExit(0)
signal.signal(signal.SIGTERM,interrupted);signal.signal(signal.SIGHUP,interrupted)
try:
 pathlib.Path(sys.argv[1]).write_text(str(p.pid))
 sys.stdin.read()
finally:
 if p.poll() is None: p.terminate()
 try: p.wait(timeout=10)
 except subprocess.TimeoutExpired: p.kill();p.wait()
 if p.returncode: raise SystemExit(p.returncode)
"""
BACKEND = """import http.server,json,sys,threading
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=b'physical-host-service'
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*args): pass
s=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
t=threading.Thread(target=s.serve_forever,daemon=True);t.start()
print(json.dumps({'event':'backend','address':'127.0.0.1:'+str(s.server_address[1])}),flush=True)
try: sys.stdin.read()
finally: s.shutdown();s.server_close();t.join()
"""

def drain(process):
    if process is None:
        return
    if process.stdin:
        process.stdin.close()
        process.stdin = None
    _, stderr = process.communicate(timeout=15)
    assert process.returncode == 0, stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ssh-host", required=True)
    parser.add_argument("--serial", help="authorized ADB serial; omit only when exactly one device is connected")
    ndk_default = os.environ.get("WIPSNET_ANDROID_NDK", str(Path.home() / "Android/Sdk/ndk/27.2.12479018"))
    parser.add_argument("--ndk", type=Path, default=Path(ndk_default))
    args = parser.parse_args()
    if args.serial is None:
        devices = [line.split()[0] for line in run(["adb", "devices"]).stdout.splitlines()[1:]
                   if len(line.split()) == 2 and line.split()[1] == "device"]
        assert len(devices) == 1, "specify --serial when there is not exactly one authorized device"
        args.serial = devices[0]
    assert args.ssh_host and not args.ssh_host.startswith("-") and all(c.isalnum() or c in ".-_@" for c in args.ssh_host)
    assert BINARY.is_file() and FIXTURE.is_file()
    android_binary = ROOT / "target/aarch64-linux-android/debug/wipsnet"
    assert android_binary.is_file(), "build and validate the current Android CLI first"
    suffix = uuid.uuid4().hex
    remote_root = "/tmp/wipsnet-physical-" + suffix
    device_root = "/data/local/tmp/wipsnet-physical-" + suffix
    remote_created = device_created = False
    fixture = forward = backend = agent = None
    local_backend = None
    reverse = None
    def remote(arguments, **kwargs):
        return run([*SSH, args.ssh_host, shlex.join([str(a) for a in arguments])], **kwargs)
    def python(code, *arguments, **kwargs):
        return remote(["python3", "-c", code, *arguments], **kwargs)
    def adb(*arguments):
        return run(["adb", "-s", args.serial, *arguments])
    def tablet(*arguments):
        return adb("shell", "-T", shlex.join([str(a) for a in arguments]))
    def client(mode, config, *extra, ok=True, input=None):
        return run(["adb", "-s", args.serial, "shell", "-T", shlex.join(["timeout", "12", device_root + "/wipsnet", mode,
            "--config", device_root + "/client.toml", *[str(a) for a in extra]])],ok=ok,input=input)
    def remote_write(path, text):
        python("import os,sys; p=sys.argv[1]; f=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600); os.write(f,sys.stdin.buffer.read());os.close(f)",path,input=text)
    def remote_cli(mode, config, *extra, **kwargs):
        return remote(["timeout", "12", remote_root + "/wipsnet", mode, "--config", config, *extra],**kwargs)
    def local_cli(mode, config, *extra, **kwargs):
        return run([BINARY, mode, "--config", config, *extra],**kwargs)
    def mutate(call, config, request, path):
        text=json.dumps(request)
        if call is remote_cli:
            remote_write(path,text)
        else:
            Path(path).write_text(text)
        call("host-mutate",config,"--request",path)
    def catalog(pin, ok=True):
        result=client("host-catalog",None,"--peer",pin,ok=ok)
        return json.loads(result.stdout) if ok else None
    def available(pin):
        deadline=time.monotonic()+30
        while True:
            result=client("host-catalog",None,"--peer",pin,ok=None)
            if result.returncode==0:
                return json.loads(result.stdout)
            assert time.monotonic()<deadline,"physical-host recovery exceeded 30 seconds"
            time.sleep(.1)
    def use(pin, ok=True):
        result=client("tunnel",None,"--peer",pin,"--service","website",ok=ok,
            input="GET / HTTP/1.0\r\nHost: fixture\r\n\r\n")
        if ok: assert "200 OK" in result.stdout and "physical-host-service" in result.stdout
    def start(call, config):
        if call is remote_cli:
            command=[*SSH,args.ssh_host,shlex.join(["python3","-c",SUPERVISOR,remote_root+"/agent.pid",remote_root+"/wipsnet","agent","--config",str(config)])]
        else:
            command=["python3","-c",SUPERVISOR,str(root/"local-agent.pid"),str(BINARY),"agent","--config",str(config)]
        process=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        try: assert event(process,process.stderr,"started")["mode"]=="independent_host"
        except BaseException: drain(process);raise
        return process
    with tempfile.TemporaryDirectory(prefix="wipsnet-physical-") as temporary:
        root=Path(temporary)
        try:
            fixture=subprocess.Popen([FIXTURE,"--ca-file",root/"relay.pem"],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            relay_url=event(fixture,fixture.stdout,"started")["relay_url"]
            port=int(relay_url.rsplit(":",1)[1].rstrip("/"))
            python("import os,sys;os.mkdir(sys.argv[1],0o700)",remote_root)
            remote_created=True
            native=root/"wipsnet"
            shutil.copyfile(BINARY,native)
            run(["strip","--strip-debug",native])
            run(["scp","-q","-o","BatchMode=yes","-o","StrictHostKeyChecking=yes",native,root/"relay.pem",args.ssh_host+":"+remote_root+"/"])
            python("import os,sys;os.chmod(sys.argv[1]+'/wipsnet',0o700)",remote_root)
            remote([remote_root+"/wipsnet","--help"])
            forward=subprocess.Popen([*SSH,"-o","ExitOnForwardFailure=yes","-N","-R",f"127.0.0.1:{port}:127.0.0.1:{port}",args.ssh_host],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            deadline=time.monotonic()+10
            while True:
                probe=python("import socket,sys;socket.create_connection(('127.0.0.1',int(sys.argv[1])),1).close()",str(port),ok=None)
                if probe.returncode==0: break
                assert forward.poll() is None and time.monotonic()<deadline,"SSH relay forwarding unavailable"
                time.sleep(.1)
            tablet("mkdir","-m","700",device_root)
            device_created=True
            stripped=root/"android-wipsnet"
            shutil.copyfile(android_binary,stripped)
            run([args.ndk/"toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip",stripped])
            adb("push",stripped,device_root+"/wipsnet")
            adb("push",root/"relay.pem",device_root+"/relay.pem")
            tablet("chmod","700",device_root+"/wipsnet")
            tablet("chmod","600",device_root+"/relay.pem")
            def config(state,ca,schema):
                return f"schema_version={schema}\ndevice_name='physical-fixture'\nstate_dir='{state}'\nlog_level='info'\n[transport]\nbind_addr='127.0.0.1:0'\nrelay_url='{relay_url}'\nrelay_only=true\nrelay_ca_file='{ca}'\n"
            device_config=root/"client.toml"
            device_config.write_text(config(device_root+"/state",device_root+"/relay.pem",1))
            adb("push",device_config,device_root+"/client.toml")
            adb("reverse","--no-rebind",f"tcp:{port}",f"tcp:{port}")
            reverse=f"tcp:{port}"
            peer=client("identity-init",None).stdout.split("Endpoint ID: ")[1].strip()
            backend=subprocess.Popen([*SSH,args.ssh_host,shlex.join(["python3","-u","-c",BACKEND])],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            remote_binding=event(backend,backend.stdout,"backend")["address"]
            local_backend_process=subprocess.Popen(["python3","-u","-c",BACKEND],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            local_backend=local_backend_process
            local_binding=event(local_backend_process,local_backend_process.stdout,"backend")["address"]
            remote_config=remote_root+"/host.toml"
            remote_write(remote_config,config(remote_root+"/state",remote_root+"/relay.pem",2))
            local_config=root/"host.toml"
            local_config.write_text(config(root/"state",root/"relay.pem",2))
            hosts=[(remote_cli,remote_config,remote_binding,remote_root+"/request.json","remote"),
                   (local_cli,local_config,local_binding,root/"request.json","local")]
            pins=[]
            for call,path,binding,request_path,label in hosts:
                pin=call("host-init",path).stdout.split("Endpoint ID: ")[1].strip()
                assert pin not in pins
                pins.append(pin)
                agent=start(call,path) # The other physical machine's fixture host is stopped.
                catalog(pin,False);use(pin,False)
                invitation_path=remote_root+"/invitation.json" if call is remote_cli else root/"invitation.json"
                issued=json.loads(call("host-invite",path,"--output",invitation_path).stdout)
                local_invitation=root/(label+"-invitation.json")
                if call is remote_cli:
                    run(["scp","-q","-o","BatchMode=yes","-o","StrictHostKeyChecking=yes",args.ssh_host+":"+str(invitation_path),local_invitation])
                else: shutil.copyfile(invitation_path,local_invitation)
                local_invitation.chmod(0o600)
                capability=json.loads(local_invitation.read_text())["code"]
                adb("push",local_invitation,device_root+"/invitation.json")
                tablet("chmod","600",device_root+"/invitation.json")
                pending=json.loads(client("pairing-request",None,"--peer",pin,"--invitation",device_root+"/invitation.json").stdout)
                assert pending=={"host":pin,"peer":peer,"invitation":issued["invitation"],"state":"pending_owner_approval"}
                saved=tablet("cat",device_root+f"/state/peer-pins/{pin}.json").stdout
                assert capability not in saved and issued["invitation"] not in saved
                tablet("rm",device_root+"/invitation.json")
                local_invitation.unlink()
                catalog(pin,False);use(pin,False)
                mutate(call,path,{"op":"approve_invitation","expected_revision":0,"invitation":issued["invitation"]},request_path)
                assert catalog(pin)["services"]==[]
                use(pin,False)
                mutate(call,path,{"op":"service","expected_revision":1,"id":"website","backend":binding,"allow":[peer]},request_path)
                before=catalog(pin)
                assert before["host"]==pin and before["services"]==[{"id":"website"}]
                use(pin)
                drain(agent);agent=None
                agent=start(call,path)
                assert available(pin)==before
                use(pin)
                fixture.send_signal(signal.SIGUSR1)
                assert event(fixture,fixture.stdout,"restarted")["relay_url"]==relay_url
                assert available(pin)==before
                use(pin)
                assert client("identity-init",None).stdout.split("Endpoint ID: ")[1].strip()==peer
                print(f"{label} physical host: tablet cold start, proof/approval, HTTP, host restart and relay rebuild passed with other host stopped",flush=True)
                drain(agent);agent=None
            remote_cli("host-invitations",remote_config)
            mutate(remote_cli,remote_config,{"op":"revoke","expected_revision":2,"peer":peer},remote_root+"/request.json")
            agent=start(remote_cli,remote_config)
            catalog(pins[0],False);use(pins[0],False)
            drain(agent);agent=None
            agent=start(local_cli,local_config)
            assert catalog(pins[1])["services"]==[{"id":"website"}]
            use(pins[1])
            print("Physical-host revocation stayed target-local; tablet pins and key survived cold process restarts",flush=True)
        finally:
            errors=[]
            for action in [lambda:drain(agent),lambda:drain(backend),lambda:drain(local_backend)]:
                try: action()
                except BaseException as error: errors.append(error)
            if reverse:
                try: adb("reverse","--remove",reverse)
                except BaseException as error: errors.append(error)
            if device_created:
                try: tablet("rm","-rf",device_root)
                except BaseException as error: errors.append(error)
            if forward:
                forward.terminate();forward.communicate(timeout=15)
            if remote_created:
                try: python("""import os,pathlib,shutil,signal,sys,time
root=pathlib.Path(sys.argv[1]); pidfile=root/'agent.pid'
if pidfile.exists():
 pid=int(pidfile.read_text()); proc=pathlib.Path('/proc')/str(pid)/'cmdline'
 def owned():
  try: return (str(root/'wipsnet').encode() in proc.read_bytes().split(b'\\0'))
  except FileNotFoundError: return False
 if owned():
  os.kill(pid,signal.SIGTERM)
  deadline=time.monotonic()+5
  while owned() and time.monotonic()<deadline: time.sleep(.1)
  if owned(): os.kill(pid,signal.SIGKILL)
shutil.rmtree(root)
""",remote_root)
                except BaseException as error: errors.append(error)
            try: stop(fixture)
            except BaseException as error: errors.append(error)
            if errors: raise RuntimeError("physical fixture cleanup failed") from errors[0]
    print("Two physical Linux hosts/Android native relay fixture passed; temporary resources removed",flush=True)

if __name__=="__main__":
    main()
