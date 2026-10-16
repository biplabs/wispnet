#!/usr/bin/env python3
"""Linux CLI lifecycle and sensitive-input regression checks; standard library only."""
import json
import os
from pathlib import Path
import select
import signal
import sqlite3
import subprocess
import tempfile

binary = Path("target/debug/wipsnet").resolve()
with tempfile.TemporaryDirectory(prefix="wipsnet-smoke-") as temporary:
    root = Path(temporary)
    config = root / "config.toml"
    def write_config(state):
        config.write_text(
            f'schema_version = 1\ndevice_name = "SECRET-SENTINEL"\n'
            f'state_dir = "{state}"\nlog_level = "debug"\n'
        )

    for mode in ("agent", "registry"):
        for stop_signal in (signal.SIGINT, signal.SIGTERM):
            state = root / f"{mode}-{stop_signal}"
            write_config(state)
            if mode == "agent":
                init = subprocess.run([binary, "identity-init", "--config", config], capture_output=True, text=True, timeout=5)
                assert init.returncode == 0, init.stderr
                public_status = init.stdout
                key_bytes = (state / "identity.key").read_bytes()
            process = subprocess.Popen(
                [binary, mode, "--config", config],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
            try:
                assert select.select([process.stderr], [], [], 10)[0], "startup timed out"
                first = process.stderr.readline()
                assert json.loads(first)["fields"]["event"] == "started", first
                assert state.stat().st_mode & 0o777 == 0o700
                # The foundation intentionally owns no listeners; future networking
                # must extend this test to assert listener cleanup.
                sockets = []
                for fd in Path(f"/proc/{process.pid}/fd").iterdir():
                    try:
                        target = os.readlink(fd)
                        if target.startswith("socket:"):
                            sockets.append(target)
                    except FileNotFoundError:
                        pass
                # Tokio uses Unix socket pairs internally for signal delivery.
                internet_inodes = set()
                for protocol in ("tcp", "tcp6", "udp", "udp6"):
                    table = Path(f"/proc/{process.pid}/net/{protocol}")
                    if table.exists():
                        internet_inodes.update(line.split()[9] for line in table.read_text().splitlines()[1:])
                assert not any(s[8:-1] in internet_inodes for s in sockets), sockets
                if mode == "agent":
                    status = subprocess.run([binary, "status", "--config", config], capture_output=True, text=True, timeout=5)
                    assert status.returncode == 0 and status.stdout == public_status
                    endpoint_id = status.stdout.split("Endpoint ID: ")[1].strip()
                    reset = subprocess.run([binary, "identity-reset", "--config", config, "--expect-id", endpoint_id], capture_output=True, text=True, timeout=5)
                    assert reset.returncode != 0 and "in use" in reset.stderr
                process.send_signal(stop_signal)
                stdout, stderr = process.communicate(timeout=5)
                assert process.returncode == 0, stderr
                events = [json.loads(line)["fields"]["event"] for line in stderr.splitlines()]
                assert events == ["stopped"], stderr
                assert "SECRET-SENTINEL" not in first + stdout + stderr
                if mode == "registry":
                    database = state / "registry.sqlite3"
                    assert database.stat().st_mode & 0o777 == 0o600
                    with sqlite3.connect(database) as db:
                        assert db.execute("PRAGMA user_version").fetchone() == (3,)
                        db.execute("UPDATE revision SET value=7 WHERE id=1")
                    restarted = subprocess.Popen(
                        [binary, mode, "--config", config], stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE, text=True,
                    )
                    try:
                        assert select.select([restarted.stderr], [], [], 10)[0]
                        line = restarted.stderr.readline()
                        assert json.loads(line)["fields"]["revision"] == 7, line
                        restarted.send_signal(stop_signal)
                        _, errors = restarted.communicate(timeout=5)
                        assert restarted.returncode == 0, errors
                    finally:
                        if restarted.poll() is None:
                            restarted.kill()
                            restarted.communicate()
                if mode == "agent":
                    assert key_bytes[8:40].hex() not in first + stdout + stderr
                    status = subprocess.run([binary, "status", "--config", config], capture_output=True, text=True, timeout=5)
                    assert status.stdout == public_status
                    reset = subprocess.run([binary, "identity-reset", "--config", config, "--expect-id", endpoint_id], capture_output=True, text=True, timeout=5)
                    assert reset.returncode == 0, reset.stderr
                    missing = subprocess.run([binary, "agent", "--config", config], capture_output=True, text=True, timeout=5)
                    assert missing.returncode != 0 and "identity missing" in missing.stderr
                    assert not (state / "identity.key").exists()
                    init = subprocess.run([binary, "identity-init", "--config", config], capture_output=True, text=True, timeout=5)
                    assert init.returncode == 0 and init.stdout != public_status
                    (state / "identity.key").write_bytes(b"PRIVATE-SENTINEL")
                    for operation in ("agent", "identity-init", "status"):
                        damaged = subprocess.run([binary, operation, "--config", config], capture_output=True, text=True, timeout=5)
                        assert damaged.returncode != 0
                        assert "PRIVATE-SENTINEL" not in damaged.stdout + damaged.stderr
                        assert (state / "identity.key").read_bytes() == b"PRIVATE-SENTINEL"
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate()

    state = root / "must-not-exist"
    for field in ("private_key", "enrollment_token", "browser_token", "http_payload"):
        write_config(state)
        with config.open("a") as output:
            output.write(f'{field} = "SECRET-SENTINEL"\n')
        result = subprocess.run([binary, "agent", "--config", config], capture_output=True, text=True, timeout=5)
        assert result.returncode != 0
        assert "SECRET-SENTINEL" not in result.stdout + result.stderr
        assert not state.exists(), "invalid config created state"

    write_config(state)
    for args in ([], ["identity-reset"], ["identity-reset", "--config", str(config)]):
        result = subprocess.run([binary, *args], capture_output=True, text=True, timeout=5)
        assert result.returncode != 0 and "panicked" not in result.stderr
    result = subprocess.run([binary, "check", "--config", config], capture_output=True, text=True, timeout=5)
    assert result.returncode == 0
    assert not state.exists(), "check must be read-only"
    shared = root / "shared"
    shared.mkdir(mode=0o755)
    write_config(shared)
    result = subprocess.run([binary, "agent", "--config", config], capture_output=True, text=True, timeout=5)
    assert result.returncode != 0
    assert "private directory" in result.stderr
print("CLI smoke checks passed")
