#!/usr/bin/env python3
"""Native per-host relay lifecycle smoke checks; --android adds authorized USB hardware.

Uses private temporary state and an ephemeral loopback TLS relay. Optional LAN
checks use a selected host UDP port. --wifi-cycle explicitly interrupts/restores
Wi-Fi and notifies one persistent endpoint. No browser UI or public relay is used.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import http.server
import ipaddress
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import signal
import subprocess
import tempfile
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target/debug/wipsnet"
FIXTURE = ROOT / "target/debug/examples/relay_fixture"


def run(arguments, *, ok=True, input=None, timeout=15):
    result = subprocess.run([str(a) for a in arguments], input=input, capture_output=True,
                            text=True, timeout=timeout)
    if ok is not None:
        assert (result.returncode == 0) == ok, result.stderr
    return result


def event(process, stream, name):
    assert select.select([stream], [], [], 10)[0], f"{name} startup timed out"
    line = stream.readline()
    assert line, f"{name} exited during startup ({process.poll()})"
    value = json.loads(line)
    fields = value.get("fields", value)
    assert fields["event"] == name, fields
    return fields


def stop(process):
    if process is None:
        return
    if process.poll() is None:
        process.send_signal(signal.SIGCONT)
        process.terminate()
    try:
        _, stderr = process.communicate(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.communicate()
        raise
    assert process.returncode == 0, stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--android", action="store_true")
    parser.add_argument("--lan-address", help="computer IPv4 address reachable over the device LAN")
    parser.add_argument("--lan-port", type=int, default=0, help="allowed host UDP port; 0 chooses an ephemeral port")
    parser.add_argument("--lan-only", action="store_true", help="run only fresh-state LAN/handoff checks")
    parser.add_argument("--wifi-cycle", action="store_true", help="briefly disable/restore device Wi-Fi while one peer-watch endpoint runs")
    parser.add_argument("--serial", help="ADB serial; required if multiple devices are authorized")
    parser.add_argument("--ndk", type=Path, default=Path(os.environ.get("WIPSNET_ANDROID_NDK", str(Path.home() / "Android/Sdk/ndk/27.2.12479018"))))
    args = parser.parse_args()
    assert BINARY.is_file() and FIXTURE.is_file(), "build wipsnet and --example relay_fixture first"
    if args.lan_address:
        assert args.android, "--lan-address requires --android"
        lan_ip = ipaddress.IPv4Address(args.lan_address)
        assert not lan_ip.is_loopback and not lan_ip.is_unspecified, "use a reachable LAN IPv4 address"
    assert 0 <= args.lan_port <= 65535, "invalid LAN UDP port"
    assert not args.lan_only or args.lan_address, "--lan-only requires --lan-address"
    assert not args.wifi_cycle or args.lan_address, "--wifi-cycle requires --lan-address"
    serial = args.serial
    if args.android:
        devices = [line.split()[0] for line in run(["adb", "devices"]).stdout.splitlines()[1:]
                   if len(line.split()) >= 2 and line.split()[1] == "device"]
        if serial is None:
            assert len(devices) == 1, "select exactly one authorized Android device with --serial"
            serial = devices[0]
        assert serial in devices, "selected Android device is not authorized"
        tools = args.ndk / "toolchains/llvm/prebuilt/linux-x86_64/bin"
        compiler = tools / "aarch64-linux-android34-clang"
        assert compiler.is_file() and (tools / "llvm-strip").is_file(), "set --ndk to a usable Android NDK"
        environment = dict(os.environ, CC_aarch64_linux_android=str(compiler),
                           AR_aarch64_linux_android=str(tools / "llvm-ar"),
                           CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=str(compiler))
        # Never validate an Android artifact left over from an older source tree.
        subprocess.run(["cargo", "build", "-p", "wipsnet-cli", "--target", "aarch64-linux-android", "--locked"],
                       cwd=ROOT, env=environment, check=True, timeout=600)

    with tempfile.TemporaryDirectory(prefix="wipsnet-relay-smoke-") as temporary:
        root = Path(temporary)
        android_root = "/data/local/tmp/wipsnet-relay-" + uuid.uuid4().hex
        mapping = None
        android_created = False
        running = fixture = None
        backend = None
        worker = None
        clients = []
        bound = {}
        watch = None
        watch_pid = None
        wifi_changed = False

        def local(mode, config, *extra, **kwargs):
            return run([BINARY, mode, "--config", config, *extra], **kwargs)

        def adb(*extra, **kwargs):
            return run(["adb", "-s", serial, *extra], **kwargs)

        def tablet(*arguments, **kwargs):
            # Bound the actual remote process too, not only its local ADB parent.
            command = " ".join(shlex.quote(str(a)) for a in ["timeout", "12", *arguments])
            return adb("shell", "-T", command, **kwargs)

        def config(name, schema, state=None, ca=None, *, bind="127.0.0.1:0", relay_only=True, relay=True):
            path = root / f"{name}.toml"
            state = state or root / name
            ca = ca or root / "relay.pem"
            text = (f"schema_version={schema}\ndevice_name='{name}'\nstate_dir='{state}'\nlog_level='info'\n"
                    f"[transport]\nbind_addr='{bind}'\nrelay_only={str(relay_only).lower()}\n")
            if relay:
                text += f"relay_url='{relay_url}'\nrelay_ca_file='{ca}'\n"
            path.write_text(text)
            return path

        def identity(result):
            return result.stdout.split("Endpoint ID: ")[1].strip()

        def status(path):
            return json.loads(local("host-status", path).stdout)

        def mutate(path, operation):
            request = root / "mutation.json"
            request.write_text(json.dumps(operation))
            local("host-mutate", path, "--request", request)

        def start(path, relay_only=True):
            process = subprocess.Popen([BINARY, "agent", "--config", path], stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True)
            try:
                fields = event(process, process.stderr, "started")
                assert fields["mode"] == "independent_host"
                if relay_only:
                    assert fields["bind_addr"] == "relay-only"
                bound[process.pid] = fields["bind_addr"]
                return process
            except BaseException:
                stop(process)
                raise

        def catalog(call, path, peer, ok=True):
            result = call("host-catalog", path, "--peer", peer, ok=ok)
            return json.loads(result.stdout) if ok else None

        def available(call, path, peer):
            # Retry only this read-only catalog probe. Never retry a service operation.
            deadline = time.monotonic() + 30
            while True:
                result = call("host-catalog", path, "--peer", peer, ok=None)
                assert time.monotonic() < deadline, "relay recovery exceeded 30-second fixture deadline"
                if result.returncode == 0:
                    return json.loads(result.stdout)
                time.sleep(0.1)

        def local_client(mode, path, *extra, **kwargs):
            return run([BINARY, mode, "--config", path, *extra], **kwargs)

        def android_client(mode, path, *extra, **kwargs):
            arguments = ["timeout", "12", android_root + "/wipsnet", mode, "--config", path, *extra]
            command = " ".join(shlex.quote(str(a)) for a in arguments)
            return run(["adb", "-s", serial, "shell", "-T", command], **kwargs)

        def use(call, path, peer, ok=True):
            response = call("tunnel", path, "--peer", peer, "--service", "website", ok=ok,
                            input="GET / HTTP/1.0\r\nHost: fixture\r\n\r\n").stdout
            if ok:
                assert "200 OK" in response and "host-local relay response" in response

        requests = []
        request_lock = threading.Lock()

        class Backend(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                with request_lock:
                    requests.append(self.path)
                body = b"host-local relay response"
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *arguments):
                pass

        try:
            fixture = subprocess.Popen([FIXTURE, "--ca-file", root / "relay.pem"],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            relay_url = event(fixture, fixture.stdout, "started")["relay_url"]
            port = int(relay_url.rsplit(":", 1)[1])
            backend = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Backend)
            worker = threading.Thread(target=backend.serve_forever, daemon=True)
            worker.start()
            binding = f"127.0.0.1:{backend.server_address[1]}"
            linux_config = config("linux-client", 1)
            linux_id = identity(local("identity-init", linux_config))
            clients.append(("Linux", local_client, linux_config, linux_id))
            if args.android:
                android_binary = ROOT / "target/aarch64-linux-android/debug/wipsnet"
                assert android_binary.is_file(), "build the Android CLI before requesting --android"
                strip = args.ndk / "toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip"
                assert strip.is_file(), "set --ndk to the Android NDK directory"
                tablet("mkdir", "-m", "700", android_root)
                android_created = True
                stripped = root / "android-wipsnet"
                shutil.copyfile(android_binary, stripped)
                run([strip, stripped])
                adb("push", stripped, android_root + "/wipsnet")
                adb("push", root / "relay.pem", android_root + "/relay.pem")
                tablet("chmod", "700", android_root + "/wipsnet")
                tablet("chmod", "600", android_root + "/relay.pem")
                device_config = config("android-client", 1, android_root + "/state", android_root + "/relay.pem")
                adb("push", device_config, android_root + "/config.toml")
                adb("reverse", "--no-rebind", f"tcp:{port}", f"tcp:{port}")
                mapping = f"tcp:{port}"
                device_config = android_root + "/config.toml"
                android_id = identity(android_client("identity-init", device_config))
                assert tablet("stat", "-c", "%a", android_root + "/state/identity.key").stdout.strip() == "600"
                assert android_id != linux_id
                clients.append(("Android native CLI/USB", android_client, device_config, android_id))

            host_configs = [config("host-a", 2), config("host-b", 2)]
            pins = [identity(local("host-init", path)) for path in host_configs]
            assert pins[0] != pins[1]
            if args.lan_only:
                for path in host_configs:
                    revision = 0
                    for name, _, _, peer in clients:
                        mutate(path, {"op": "approve", "expected_revision": revision, "peer": peer, "name": name})
                        revision += 1
                    mutate(path, {"op": "service", "expected_revision": revision, "id": "website", "backend": binding,
                                  "allow": [client[3] for client in clients]})
            else:
                snapshots = []
                for index, (path, pin) in enumerate(zip(host_configs, pins)):
                    running = start(path)  # The other host has no running process.
                    before_count = len(requests)
                    for _, call, client_path, _ in clients:
                        catalog(call, client_path, pin, ok=False)
                        use(call, client_path, pin, ok=False)
                    assert len(requests) == before_count, "unapproved client reached a backend"
                    revision = 0
                    for name, _, _, peer in clients:
                        mutate(path, {"op": "approve", "expected_revision": revision, "peer": peer, "name": name})
                        revision += 1
                    mutate(path, {"op": "service", "expected_revision": revision, "id": "website", "backend": binding,
                                  "allow": [client[3] for client in clients]})
                    for name, call, client_path, peer in clients:
                        listing = available(call, client_path, pin)
                        assert listing["host"] == pin and listing["services"] == [{"id": "website"}]
                        use(call, client_path, pin)
                        count = len(requests)
                        catalog(call, client_path, pins[1 - index], ok=False)
                        assert len(requests) == count, "wrong peer pin reached the live host backend"
                        assert identity(call("identity-init", client_path)) == peer
                        print(f"{name}: host {index + 1} independently approved catalog/service through pinned-ID relay", flush=True)
                    before = status(path)
                    stop(running)
                    running = start(path)
                    assert status(path) == before
                    for _, call, client_path, _ in clients:
                        assert available(call, client_path, pin) == listing
                        use(call, client_path, pin)
                    snapshots.append(before)
                    stop(running)
                    running = None
                assert snapshots[0]["generation"] != snapshots[1]["generation"]
                # A stays offline while B serves a device; a separate Linux endpoint
                # probes A concurrently, so CLI identity leases are not shared.
                running = start(host_configs[1])
                name, call, client_path, _ = clients[-1]
                available(call, client_path, pins[1])
                with ThreadPoolExecutor(max_workers=2) as pool:
                    offline = pool.submit(catalog, local_client, linux_config, pins[0], False)
                    started = time.monotonic()
                    if call is local_client:
                        # A separate temporary client key avoids a duplicate key lease.
                        fresh = config("parallel-client", 1)
                        peer = identity(local("identity-init", fresh))
                        before = status(host_configs[1])["revision"]
                        mutate(host_configs[1], {"op": "approve", "expected_revision": before, "peer": peer, "name": "parallel"})
                        result = catalog(local_client, fresh, pins[1])
                    else:
                        result = catalog(call, client_path, pins[1])
                    assert result["host"] == pins[1]
                    assert time.monotonic() - started < 4, "healthy catalog waited on unrelated offline target"
                    offline.result(timeout=10)
                print(f"{name}: healthy host works while an independent endpoint times out on the offline host", flush=True)
                before = catalog(call, client_path, pins[1])
                count = len(requests)
                stop(running)
                running = None
                fixture.send_signal(signal.SIGSTOP)
                try:
                    started = time.monotonic()
                    running = start(host_configs[1])
                    assert time.monotonic() - started < 2, "local host startup waited for the relay"
                    assert status(host_configs[1])["host"] == pins[1]
                    catalog(call, client_path, pins[1], ok=False)
                    assert len(requests) == count
                finally:
                    fixture.send_signal(signal.SIGCONT)
                assert available(call, client_path, pins[1]) == before
                fixture.send_signal(signal.SIGUSR1)
                assert event(fixture, fixture.stdout, "restarted")["relay_url"] == relay_url
                assert available(call, client_path, pins[1]) == before
                use(call, client_path, pins[1])
                print(f"{name}: relay outage/restart preserved pins, catalog and host approvals", flush=True)
                # Target-local revocation survives host restart and has no effect on A.
                before_revision = status(host_configs[1])["revision"]
                revoked_peer = clients[-1][3]
                mutate(host_configs[1], {"op": "revoke", "expected_revision": before_revision, "peer": revoked_peer})
                catalog(call, client_path, pins[1], ok=False)
                use(call, client_path, pins[1], ok=False)
                stop(running)
                running = start(host_configs[1])
                catalog(call, client_path, pins[1], ok=False)
                stop(running)
                running = start(host_configs[0])
                available(call, client_path, pins[0])
                use(call, client_path, pins[0])
                print(f"{name}: revocation stayed host-local and persisted across restart", flush=True)

            if args.lan_address:
                assert "inet " in tablet("ip", "-o", "-4", "addr", "show", "wlan0").stdout, "connect the Android device to the LAN first"
                stop(running)
                lan_host = config("lan-host", 2, root / "host-a", bind=f"{args.lan_address}:{args.lan_port}", relay_only=False)
                running = start(lan_host, relay_only=False)
                address = bound[running.pid]
                assert address.rsplit(":", 1)[0] == args.lan_address
                direct = config("android-direct", 1, android_root + "/state", bind="0.0.0.0:0", relay_only=False, relay=False)
                hybrid = config("android-hybrid", 1, android_root + "/state", android_root + "/relay.pem", bind="0.0.0.0:0", relay_only=False)
                adb("push", direct, android_root + "/direct.toml")
                adb("push", hybrid, android_root + "/hybrid.toml")
                direct_path = android_root + "/direct.toml"
                hybrid_path = android_root + "/hybrid.toml"
                report = json.loads(android_client("peer-probe", direct_path, "--peer", pins[0], "--address", address).stdout)
                assert report["ok"] and report["selected_path"] == "direct" and report["relay_status"] == "disabled"
                assert report["host"] == pins[0] and report["client"] == android_id

                def direct_service():
                    result = android_client("tunnel", direct_path, "--peer", pins[0], "--address", address, "--service", "website",
                                            input="GET / HTTP/1.0\r\nHost: fixture\r\n\r\n")
                    assert "200 OK" in result.stdout and "host-local relay response" in result.stdout

                direct_service()
                print("Android LAN: selected direct path and actual HTTP bytes with client relays disabled", flush=True)
                if args.wifi_cycle:
                    arguments = [android_root + "/wipsnet", "peer-watch", "--config", hybrid_path, "--peer", pins[0], "--address", address]
                    command = "echo $$; exec " + " ".join(shlex.quote(a) for a in arguments)
                    command = "timeout 180 sh -c " + shlex.quote(command)
                    watch = subprocess.Popen(["adb", "-s", serial, "shell", "-T", command], stdin=subprocess.DEVNULL,
                                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    assert select.select([watch.stdout], [], [], 10)[0]
                    watch_pid = watch.stdout.readline().strip()
                    assert watch_pid.isdigit()

                    def path_sample(expected, seconds=30):
                        deadline = time.monotonic() + seconds
                        while time.monotonic() < deadline:
                            if not select.select([watch.stdout], [], [], min(2, deadline - time.monotonic()))[0]:
                                continue
                            line = watch.stdout.readline()
                            assert line, f"peer-watch exited: {watch.poll()}"
                            sample = json.loads(line)
                            assert sample["client"] == android_id and sample["host"] == pins[0]
                            print(f"Android watch: ok={sample['ok']} path={sample.get('selected_path', 'unavailable')} relay={sample['relay_status']}", flush=True)
                            if sample["ok"] and sample["selected_path"] == expected:
                                assert sample["generation"] == report["generation"] and sample["revision"] == report["revision"]
                                return sample
                        raise AssertionError(f"peer-watch did not recover {expected} within {seconds}s")

                    path_sample("direct")
                    wifi_changed = True
                    tablet("svc", "wifi", "disable")
                    deadline = time.monotonic() + 15
                    while "inet " in tablet("ip", "-o", "-4", "addr", "show", "wlan0").stdout:
                        assert time.monotonic() < deadline, "Wi-Fi did not disconnect"
                        time.sleep(0.2)
                    tablet("kill", "-USR1", watch_pid)
                    path_sample("relay")
                    tablet("svc", "wifi", "enable")
                    deadline = time.monotonic() + 45
                    while "inet " not in tablet("ip", "-o", "-4", "addr", "show", "wlan0").stdout:
                        assert time.monotonic() < deadline, "Wi-Fi did not restore"
                        time.sleep(0.5)
                    tablet("kill", "-USR1", watch_pid)
                    path_sample("direct")
                    tablet("kill", "-TERM", watch_pid)
                    _, errors = watch.communicate(timeout=10)
                    assert watch.returncode == 0, errors
                    watch = None
                    watch_pid = None
                    wifi_changed = False
                    direct_service()
                    assert identity(android_client("identity-init", device_config)) == android_id
                    print("Android handoff: one peer-watch process/endpoint selected direct -> relay -> direct; identity and catalog unchanged", flush=True)
        finally:
            # Attempt every owned cleanup even if a process/device disappeared.
            actions = [lambda: stop(running), lambda: stop(fixture)]
            if wifi_changed:
                def restore_wifi():
                    tablet("svc", "wifi", "enable")
                    deadline = time.monotonic() + 45
                    while "inet " not in tablet("ip", "-o", "-4", "addr", "show", "wlan0").stdout:
                        assert time.monotonic() < deadline, "failed to restore Wi-Fi after test"
                        time.sleep(0.5)
                actions.insert(0, restore_wifi)
            if watch_pid:
                actions.insert(0, lambda: tablet("kill", "-TERM", watch_pid, ok=None))
            if watch:
                actions.insert(1, lambda: watch.communicate(timeout=10))
            if args.android and mapping:
                actions.insert(1, lambda: adb("reverse", "--remove", mapping))
            if args.android and android_created:
                actions.insert(1, lambda: tablet("rm", "-rf", android_root))
            if backend:
                actions.extend([backend.shutdown, backend.server_close])
            if worker:
                actions.append(lambda: worker.join(timeout=5))
            errors = []
            for action in actions:
                try:
                    action()
                except Exception as error:
                    errors.append(error)
            if errors:
                raise ExceptionGroup("relay fixture cleanup failed", errors)
    scope = ""
    if args.android:
        scope = " (authorized Android CLI"
        scope += ", LAN direct" if args.lan_address else " over USB"
        scope += ", Wi-Fi/USB-relay handoff" if args.wifi_cycle else ""
        scope += ")"
    print("Native relay lifecycle smoke checks passed" + scope)


if __name__ == "__main__":
    main()
