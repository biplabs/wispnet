#!/usr/bin/env python3
"""Real-process HTTPS enrollment, policy, restart and backup/restore checks.
Use --android for an additional native Android CLI check over temporary USB reverse.
"""
import argparse
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import signal
import socket
import sqlite3
import ssl
import subprocess
import tempfile
import threading
import uuid

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--android", action="store_true")
args = parser.parse_args()
binary = Path("target/debug/wispnet").resolve()
android_binary = Path("target/aarch64-linux-apps/android/debug/wispnet").resolve()

def run(command, **kwargs):
    result = subprocess.run(command, capture_output=True, text=True, timeout=30, **kwargs)
    assert result.returncode == 0, result.stderr
    return result.stdout

def command(*arguments):
    return run([binary, *arguments])

def start(config):
    process = subprocess.Popen([binary, "registry", "--config", config], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        assert select.select([process.stderr], [], [], 10)[0], "registry startup timed out"
        fields = json.loads(process.stderr.readline())["fields"]
        assert fields["event"] == "started" and fields["networking"] is True
        return process
    except BaseException:
        process.kill()
        process.communicate()
        raise

def stop(process):
    process.send_signal(signal.SIGTERM)
    stdout, stderr = process.communicate(timeout=15)
    assert process.returncode == 0, stderr
    assert not stdout
    assert [json.loads(line)["fields"]["event"] for line in stderr.splitlines()] == ["stopped"]

with tempfile.TemporaryDirectory(prefix="wispnet-registry-") as temporary:
    root = Path(temporary)
    state = root / "state"
    config = root / "registry.toml"
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    config.write_text(f'schema_version=1\ndevice_name="controller"\nstate_dir="{state}"\nlog_level="info"\n[registry]\nbind_addr="127.0.0.1:{port}"\nmesh="test"\n')
    output = command("registry-init", "--config", config)
    owner = (state / "owner.token").read_text()
    assert owner not in output
    assert (state / "owner.token").stat().st_mode & 0o777 == 0o600
    assert command("registry-init", "--config", config) == output
    certificate, key = root / "root.pem", root / "tls.key"
    run(["openssl", "req", "-x509", "-newkey", "ed25519", "-nodes", "-days", "1", "-subj", "/CN=localhost",
         "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1", "-addext", "basicConstraints=critical,CA:FALSE", "-keyout", key, "-out", certificate])
    counts = {"/v1/enrollment/challenge": 0, "/v1/enrollment/proof": 0}

    class Proxy(BaseHTTPRequestHandler):
        def do_GET(self):
            self.forward()
        def do_POST(self):
            self.forward()
        def log_message(self, *arguments):
            pass
        def forward(self):
            if self.path in counts:
                counts[self.path] += 1
            size = int(self.headers.get("Content-Length", "0"))
            assert size <= 128 * 1024
            body = self.rfile.read(size) if size else None
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
            try:
                connection.request(self.command, self.path, body=body, headers=dict(self.headers))
                response = connection.getresponse()
                payload = response.read()
                self.send_response(response.status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.send_header("Cache-Control", response.getheader("Cache-Control", "no-store"))
                self.end_headers()
                self.wfile.write(payload)
            finally:
                connection.close()

    proxy = ThreadingHTTPServer(("127.0.0.1", 0), Proxy)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certificate, key)
    proxy.socket = context.wrap_socket(proxy.socket, server_side=True)
    tls_port = proxy.server_address[1]
    origin = f"https://127.0.0.1:{tls_port}/"
    thread = threading.Thread(target=proxy.serve_forever, daemon=True)
    thread.start()
    trusted = ssl.create_default_context(cafile=certificate)
    android_root = "/data/local/tmp/wispnet-enrollment-" + uuid.uuid4().hex
    reversed_port = False
    process = None

    def adb_shell(*arguments):
        return run(["adb", "-d", "shell", " ".join(shlex.quote(str(a)) for a in arguments)])

    def call(method, path, token=None, payload=None):
        connection = http.client.HTTPSConnection("127.0.0.1", tls_port, context=trusted, timeout=10)
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = f"Bearer {token}"
        connection.request(method, path, body=json.dumps(payload) if payload is not None else None, headers=headers)
        response = connection.getresponse()
        status, cache = response.status, response.getheader("Cache-Control")
        data = json.loads(response.read())
        connection.close()
        assert cache == "no-store"
        return status, data

    def listing():
        status, data = call("GET", "/v1/enrollment", owner)
        assert status == 200
        return data

    def enroll_device(android=False):
        platform = "apps/android" if android else "linux"
        invitation_file = root / (platform + "-invitation.json")
        invite_request = root / (platform + "-invite-request.json")
        invite_request.write_text('{"op":"invite","ttl_seconds":600}')
        admin_output = run(["python3", "scripts/registry_admin.py", "--origin", origin, "--owner-token", state / "owner.token",
                            "--mutation", invite_request, "--invitation-out", invitation_file],
                           env={**os.environ, "SSL_CERT_FILE": str(certificate)})
        invitation = json.loads(invitation_file.read_text())
        assert invitation["code"] not in admin_output
        assert invitation_file.stat().st_mode & 0o777 == 0o600
        device_config = root / (platform + ".toml")
        device_state = root / platform
        path_state = f"{android_root}/state" if android else str(device_state)
        path_ca = f"{android_root}/root.pem" if android else str(certificate)
        device_config.write_text(f'schema_version=1\ndevice_name="test-{platform}"\nstate_dir="{path_state}"\nlog_level="info"\n[controller]\norigin="{origin}"\ncredential_file="{path_state}/device.token"\nca_certificate_file="{path_ca}"\n')
        if android:
            run(["adb", "-d", "push", device_config, f"{android_root}/config.toml"])
            run(["adb", "-d", "push", invitation_file, f"{android_root}/invitation.json"])
            adb_shell("chmod", "600", f"{android_root}/invitation.json")
            def device_command(mode):
                arguments = [f"{android_root}/wispnet", mode, "--config", f"{android_root}/config.toml"]
                if mode == "enroll":
                    arguments += ["--invitation", f"{android_root}/invitation.json"]
                return adb_shell(*arguments)
        else:
            def device_command(mode):
                arguments = [mode, "--config", device_config]
                if mode == "enroll":
                    arguments += ["--invitation", invitation_file]
                return command(*arguments)
        first = device_command("enroll")
        assert "Pending owner approval" in first and invitation["code"] not in first
        records = listing()
        pending = next(i for i in records["invitations"] if i["invitation"] == invitation["invitation"])
        assert pending["state"] == "pending" and pending["name"] == f"test-{platform}"
        identity = pending["device"]
        assert identity in first
        assert call("POST", "/v1/enrollment/admin", owner, {"op": "approve", "invitation": invitation["invitation"],
                    "device": identity, "name": f"test-{platform}", "expected_revision": records["revision"]})[0] == 200
        second = device_command("enroll")
        assert "Enrolled device:" in second
        paired_counts = counts.copy()
        assert "Already enrolled" in device_command("enroll")
        assert "Authorized services:" in device_command("registry-directory")
        assert counts == paired_counts, "restart repeated pairing"
        if android:
            # Native Android shell CLI, not the Android companion UI/Keystore (#12).
            assert adb_shell("stat", "-c", "%a", f"{path_state}/device.token").strip() == "600"
            # Exercise cross-process lease/reset protection on the actual OS.
            remote_agent = f"echo $$; exec {shlex.quote(android_root + '/wispnet')} agent --config {shlex.quote(android_root + '/config.toml')}"
            agent = subprocess.Popen(["adb", "-d", "shell", remote_agent], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            remote_pid = None
            try:
                assert select.select([agent.stdout], [], [], 10)[0]
                remote_pid = agent.stdout.readline().strip()
                assert remote_pid.isdigit()
                assert select.select([agent.stderr], [], [], 10)[0]
                boot = agent.stderr.readline()
                started = agent.stderr.readline()
                assert json.loads(boot)["fields"]["event"] == "registry_bootstrap", boot
                assert json.loads(started)["fields"]["event"] == "started", started
                reset_args = [android_root + "/wispnet", "identity-reset", "--config", android_root + "/config.toml", "--expect-id", identity]
                refused = subprocess.run(["adb", "-d", "shell", " ".join(shlex.quote(a) for a in reset_args)], capture_output=True, text=True, timeout=10)
                assert refused.returncode != 0 and "in use" in refused.stderr
                adb_shell("kill", "-TERM", remote_pid)
                stdout, stderr = agent.communicate(timeout=10)
                assert agent.returncode == 0 and not stdout
                assert json.loads(stderr)["fields"]["event"] == "stopped", stderr
            finally:
                if agent.poll() is None:
                    if remote_pid and remote_pid.isdigit():
                        adb_shell("kill", "-KILL", remote_pid)
                    agent.kill()
                    agent.communicate()
            assert "Authorized services:" in device_command("registry-directory")
            assert counts == paired_counts
            print("Native Android HTTPS enrollment, persistent restart and exclusive identity lease checks passed")
            return identity, None
        token = (device_state / "device.token").read_text()
        assert token not in first + second and (device_state / "device.token").stat().st_mode & 0o777 == 0o600
        key_before = (device_state / "identity.key").read_bytes()
        for _ in range(2):
            agent = subprocess.Popen([binary, "agent", "--config", device_config], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                assert select.select([agent.stderr], [], [], 10)[0]
                boot = agent.stderr.readline()
                started = agent.stderr.readline()
                assert json.loads(boot)["fields"]["event"] == "registry_bootstrap", boot
                assert json.loads(started)["fields"]["event"] == "started", started
                agent.send_signal(signal.SIGTERM)
                stdout, stderr = agent.communicate(timeout=5)
                assert agent.returncode == 0 and not stdout
                assert json.loads(stderr)["fields"]["event"] == "stopped"
                assert token not in boot + started + stderr
            finally:
                if agent.poll() is None:
                    agent.kill()
                    agent.communicate()
        assert counts == paired_counts and (device_state / "identity.key").read_bytes() == key_before
        return identity, token

    try:
        process = start(config)
        assert call("GET", "/readyz")[0] == 200
        assert call("GET", "/v1/directory")[0] == 401
        identity, device_token = enroll_device()
        if args.android:
            assert android_binary.exists(), "build aarch64-linux-android CLI first"
            # Refuse to replace a previously configured USB reverse mapping.
            mappings = run(["adb", "-d", "reverse", "--list"])
            assert f"tcp:{tls_port} " not in mappings
            adb_shell("mkdir", "-m", "700", android_root)
            stripped = root / "android-wispnet"
            shutil.copyfile(android_binary, stripped)
            strip = Path.home() / "Android/Sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip"
            run([strip, stripped])
            run(["adb", "-d", "push", stripped, f"{android_root}/wispnet"])
            run(["adb", "-d", "push", certificate, f"{android_root}/root.pem"])
            adb_shell("chmod", "644", f"{android_root}/root.pem")
            adb_shell("chmod", "700", f"{android_root}/wispnet")
            run(["adb", "-d", "reverse", f"tcp:{tls_port}", f"tcp:{tls_port}"])
            reversed_port = True
            enroll_device(android=True)
        revision = listing()["revision"]
        assert call("POST", "/v1/mutations", device_token, {"op": "revoke", "expected_revision": revision, "device": identity})[0] == 403
        assert call("POST", "/v1/mutations", owner, {"op": "service", "expected_revision": revision, "id": "website", "host": identity, "grants": []})[0] == 200
        assert call("POST", "/v1/mutations", device_token, {"op": "publish", "expected_revision": revision + 1, "id": "website", "address": "127.0.0.1:7779"})[0] == 200
        expected = call("GET", "/v1/directory", device_token)[1]
        backup = root / "backup.sqlite3"
        command("registry-backup", "--config", config, "--output", backup)
        assert backup.stat().st_mode & 0o777 == 0o600
        assert owner.encode() not in backup.read_bytes() and device_token.encode() not in backup.read_bytes()
        stop(process)
        process = None
        restored = root / "restored"
        restored.mkdir(mode=0o700)
        shutil.copy2(backup, restored / "registry.sqlite3")
        shutil.copy2(state / "owner.token", restored / "owner.token")
        config.write_text(config.read_text().replace(str(state), str(restored)))
        process = start(config)
        assert call("GET", "/v1/directory", device_token)[1] == expected
        assert call("POST", "/v1/mutations", owner, {"op": "revoke", "expected_revision": expected["revision"], "device": identity})[0] == 200
        assert call("GET", "/v1/directory", device_token)[0] == 401
        revoked = next(d for d in listing()["devices"] if d["device"] == identity)
        assert revoked["state"] == "revoked" and revoked["revoked_at"] is not None
        with sqlite3.connect(restored / "registry.sqlite3") as db:
            assert db.execute("PRAGMA user_version").fetchone() == (14,)
            assert db.execute("SELECT value FROM revision").fetchone() == (expected["revision"] + 1,)
    finally:
        if process and process.poll() is None:
            stop(process)
        proxy.shutdown()
        proxy.server_close()
        thread.join(timeout=5)
        if args.android:
            if reversed_port:
                run(["adb", "-d", "reverse", "--remove", f"tcp:{tls_port}"])
            adb_shell("rm", "-rf", android_root)
print("HTTPS enrollment, persistent agent restart, registry policy and restore checks passed")
