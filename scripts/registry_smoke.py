#!/usr/bin/env python3
"""Real-process loopback registry checks; public deployment requires HTTPS proxy."""
import http.client
import json
from pathlib import Path
import select
import shutil
import signal
import socket
import sqlite3
import subprocess
import tempfile

binary = Path("target/debug/wipsnet").resolve()

def command(*args):
    result = subprocess.run([binary, *args], capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, result.stderr
    return result.stdout

def start(config):
    process = subprocess.Popen([binary, "registry", "--config", config], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        assert select.select([process.stderr], [], [], 10)[0], "registry startup timed out"
        first = process.stderr.readline()
        fields = json.loads(first)["fields"]
        assert fields["event"] == "started" and fields["networking"] is True, first
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

with tempfile.TemporaryDirectory(prefix="wipsnet-registry-") as temporary:
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
    identity_config = root / "identity.toml"
    identity_config.write_text(f'schema_version=1\ndevice_name="host"\nstate_dir="{root / "host"}"\nlog_level="info"\n')
    identity = command("identity-init", "--config", identity_config).split("Endpoint ID: ")[1].strip()

    def call(method, path, token=None, payload=None):
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
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

    process = start(config)
    try:
        assert call("GET", "/readyz")[0] == 200
        assert call("GET", "/v1/directory")[0] == 401
        assert call("POST", "/v1/mutations", payload={"op": "enroll"})[0] == 401
        status, enrolled = call("POST", "/v1/mutations", owner, {"op": "enroll", "expected_revision": 0, "device": identity})
        assert status == 200 and enrolled["revision"] == 1
        device_token = enrolled["token"]
        assert call("POST", "/v1/mutations", device_token, {"op": "revoke", "expected_revision": 1, "device": identity})[0] == 403
        assert call("POST", "/v1/mutations", owner, {"op": "service", "expected_revision": 1, "id": "website", "host": identity, "grants": []})[0] == 200
        assert call("POST", "/v1/mutations", device_token, {"op": "publish", "expected_revision": 2, "id": "website", "address": "127.0.0.1:7779"})[0] == 200
        expected = call("GET", "/v1/directory", device_token)[1]
        assert expected["revision"] == 3 and expected["services"][0]["address"] == "127.0.0.1:7779"
        backup = root / "backup.sqlite3"
        command("registry-backup", "--config", config, "--output", backup)
        assert backup.stat().st_mode & 0o777 == 0o600
        raw = backup.read_bytes()
        assert owner.encode() not in raw and device_token.encode() not in raw
    finally:
        if process.poll() is None:
            stop(process)
    # Restore into a fresh private state directory and run a new registry process.
    restored = root / "restored"
    restored.mkdir(mode=0o700)
    shutil.copy2(backup, restored / "registry.sqlite3")
    shutil.copy2(state / "owner.token", restored / "owner.token")
    config.write_text(config.read_text().replace(str(state), str(restored)))
    process = start(config)
    try:
        assert call("GET", "/v1/directory", device_token)[1] == expected
        assert call("GET", "/v1/directory", owner)[0] == 200
        assert call("POST", "/v1/mutations", owner, {"op": "revoke", "expected_revision": 3, "device": identity})[0] == 200
        assert call("GET", "/v1/directory", device_token)[0] == 401
    finally:
        if process.poll() is None:
            stop(process)
    with sqlite3.connect(restored / "registry.sqlite3") as db:
        assert db.execute("PRAGMA user_version").fetchone() == (2,)
        assert db.execute("SELECT value FROM revision").fetchone() == (4,)
print("Registry API, CLI bootstrap, live backup and process restore checks passed")
