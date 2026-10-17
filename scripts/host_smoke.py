#!/usr/bin/env python3
"""Native independent-host/owner/catalog/service regression checks."""
import base64
import http.server
import json
from pathlib import Path
import select
import signal
import subprocess
import tempfile
import threading

binary = Path("target/debug/wipsnet").resolve()

def run(*args, ok=True, input=None):
    result = subprocess.run([binary, *args], input=input, capture_output=True, text=True, timeout=15)
    assert (result.returncode == 0) == ok, result.stderr
    return result

with tempfile.TemporaryDirectory(prefix="wipsnet-hosts-") as temporary:
    root = Path(temporary)
    def configuration(name, host=True):
        path = root / f"{name}.toml"
        state = root / name
        path.write_text(f'schema_version={2 if host else 1}\ndevice_name="{name}"\nstate_dir="{state}"\nlog_level="info"\n'
                        + ('[transport]\nbind_addr="127.0.0.1:0"\n' if host else ''))
        return path, state
    a, a_state = configuration("host-a")
    b, b_state = configuration("host-b")
    client, client_state = configuration("client", host=False)
    fresh, _ = configuration("fresh", host=False)
    client_id = run("identity-init", "--config", client).stdout.split("Endpoint ID: ")[1].strip()
    fresh_id = run("identity-init", "--config", fresh).stdout.split("Endpoint ID: ")[1].strip()
    a_id = run("host-init", "--config", a).stdout.split("Endpoint ID: ")[1].strip()
    b_id = run("host-init", "--config", b).stdout.split("Endpoint ID: ")[1].strip()
    code_config, _ = configuration("code-host")
    with code_config.open("a") as document:
        document.write('relay_url="https://relay.example"\n')
    code_host = run("host-init", "--config", code_config).stdout.split("Endpoint ID: ")[1].strip()
    code_path = root / "invitation.txt"
    result = run("host-invite", "--config", code_config, "--output", code_path, "--format", "code")
    encoded = code_path.read_text()
    assert encoded.startswith("wispnet1:") and code_path.stat().st_mode & 0o077 == 0
    payload = encoded.removeprefix("wispnet1:")
    invitation = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))
    assert invitation["host"] == code_host and invitation["scope"] == "request_access"
    assert invitation["host_name"] == subprocess.check_output(["hostname"], text=True).strip()
    assert invitation["code"] not in result.stdout and encoded not in result.stdout
    run("host-invite", "--config", code_config, "--output", code_path, "--format", "code", ok=False)
    assert a_id != b_id
    assert (a_state / "host-owner.token").read_bytes() != (b_state / "host-owner.token").read_bytes()
    def status(config):
        return json.loads(run("host-status", "--config", config).stdout)
    def mutate(config, operation):
        path = root / "operation.json"
        path.write_text(json.dumps(operation))
        return run("host-mutate", "--config", config, "--request", path)
    def start(config):
        process = subprocess.Popen([binary, "agent", "--config", config], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            assert select.select([process.stderr], [], [], 10)[0]
            fields = json.loads(process.stderr.readline())["fields"]
            assert fields["event"] == "started" and fields["mode"] == "independent_host", fields
            return process, fields["bind_addr"]
        except BaseException:
            process.kill()
            process.communicate()
            raise
    def stop(process):
        process.send_signal(signal.SIGTERM)
        stdout, stderr = process.communicate(timeout=5)
        assert process.returncode == 0 and not stdout, stderr
        assert json.loads(stderr)["fields"]["event"] == "stopped"
    def catalog(config, host, address, ok=True):
        result = run("host-catalog", "--config", config, "--peer", host, "--address", address, ok=ok)
        return json.loads(result.stdout) if ok else None
    class Backend(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            data = b"independent host response"
            self.send_response(200)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        def log_message(self, *args):
            pass
    backend = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Backend)
    worker = threading.Thread(target=backend.serve_forever, daemon=True)
    worker.start()
    binding = f"127.0.0.1:{backend.server_address[1]}"
    def use(config, host, address):
        response = run("tunnel", "--config", config, "--peer", host, "--address", address, "--service", "llama",
                       input="GET / HTTP/1.0\r\nHost: local\r\n\r\n").stdout
        assert "200 OK" in response and "independent host response" in response
    running = None
    try:
        # Symmetric cold-start with the other host absent. Local owner operations
        # while running feed the same endpoint's catalogs and service decisions.
        for config, host_id, other_config in [(a, a_id, b), (b, b_id, a)]:
            running, address = start(config)
            catalog(client, host_id, address, ok=False)
            mutate(config, {"op": "approve", "expected_revision": 0, "peer": client_id, "name": "client"})
            mutate(config, {"op": "service", "expected_revision": 1, "id": "llama", "backend": binding, "allow": [client_id]})
            listing = catalog(client, host_id, address)
            assert listing["host"] == host_id and listing["revision"] == 2 and [s["id"] for s in listing["services"]] == ["llama"]
            use(client, host_id, address)
            mutate(config, {"op": "approve", "expected_revision": 2, "peer": fresh_id, "name": "fresh client"})
            mutate(config, {"op": "service", "expected_revision": 3, "id": "llama", "backend": binding, "allow": [client_id, fresh_id]})
            use(fresh, host_id, address)
            before = status(config)
            stop(running)
            running, address = start(config)
            assert status(config) == before
            use(client, host_id, address)
            mutate(config, {"op": "revoke", "expected_revision": 4, "peer": fresh_id})
            catalog(fresh, host_id, address, ok=False)
            use(client, host_id, address)
            stop(running)
            running = None
        assert status(a)["revision"] == status(b)["revision"] == 5
        assert status(a)["generation"] != status(b)["generation"]
        # Corrupt A affects only A. Initialization must not replace lost authority.
        (a_state / "host.sqlite3").write_bytes(b"PRIVATE-SENTINEL")
        failure = run("agent", "--config", a, ok=False)
        assert "PRIVATE-SENTINEL" not in failure.stderr
        run("host-init", "--config", a, ok=False)
        running, address = start(b)
        use(client, b_id, address)
        stop(running)
        running = None
        (b_state / "host.sqlite3").unlink()
        run("agent", "--config", b, ok=False)
        run("host-init", "--config", b, ok=False)
        assert not (b_state / "host.sqlite3").exists()
    finally:
        if running and running.poll() is None:
            stop(running)
        backend.shutdown()
        backend.server_close()
        worker.join(timeout=5)
print("Symmetric independent host startup, control/service, owner changes, failure isolation checks passed")
