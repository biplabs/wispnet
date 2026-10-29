#!/usr/bin/env python3
"""Run MCP commands over two isolated Wispnet nodes and native Shell streams.

Temporary identities, HOME directories, policies, and loopback discovery hints
keep these checks independent of installed nodes and real device permissions.
"""

import base64
import http.client
import json
import os
from pathlib import Path
import shlex
import socket
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/wispnet"
sys.path.insert(0, str(ROOT / "apps/node/desktop"))
from wispnet_desktop.api import NodeAPI


class MCPExecTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wispnet-mcp-exec-")
        self.root = Path(self.temporary.name).resolve()
        self.processes = []
        self.request_id = 0

    def tearDown(self):
        try:
            for process in self.processes:
                process.terminate()
                try:
                    process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        finally:
            self.temporary.cleanup()

    def host(self, name):
        home = self.root / (name + "-home")
        home.mkdir(mode=0o700)
        environment = dict(os.environ, HOME=str(home), SHELL="/bin/sh", ENV="")
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.bind(("127.0.0.1", 0))
            port = udp.getsockname()[1]
        config, state = self.root / (name + ".toml"), self.root / name
        config.write_text(
            f'schema_version=2\ndevice_name="{name}"\nstate_dir="{state}"\nlog_level="info"\n'
            f'[transport]\nbind_addr="127.0.0.1:{port}"\nrelay_url="https://relay.example"\n'
        )
        initialized = subprocess.run([BINARY, "host-init", "--config", config],
                                     capture_output=True, text=True, check=True,
                                     env=environment, timeout=15)
        peer = initialized.stdout.split("Endpoint ID: ")[1].strip()
        daemon = subprocess.Popen([BINARY, "agent", "--config", config],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                  env=environment)
        self.processes.append(daemon)
        deadline = time.monotonic() + 12
        api = NodeAPI(config)
        while True:
            self.assertIsNone(daemon.poll(), "Fixture node exited before its API was ready")
            try:
                api.request(op="status")
                break
            except (FileNotFoundError, ConnectionRefusedError):
                self.assertLess(time.monotonic(), deadline, "Fixture node did not start")
                time.sleep(.03)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            registry_port = listener.getsockname()[1]
        api.request(op="configure_access", settings=dict(
            registry_port=registry_port, allowed_origins=[], service_ports=[]))
        api.request(op="enable")
        return dict(config=config, api=api, peer=peer, port=port, home=home,
                    environment=environment, registry_port=registry_port)

    def mutate(self, host, **operation):
        api = host["api"]
        operation["expected_revision"] = api.request(op="status")["host"]["revision"]
        return api.request(op="mutate", operation=operation)

    def pair(self, client, target):
        invitation_path = self.root / (target["peer"] + ".json")
        subprocess.run([BINARY, "host-invite", "--config", target["config"],
                        "--output", invitation_path], capture_output=True, check=True,
                       env=target["environment"], timeout=15)
        invitation = json.loads(invitation_path.read_text())
        invitation["direct_hint"] = f'127.0.0.1:{target["port"]}'
        client["api"].request(op="pair", host=target["peer"], invitation=json.dumps(invitation))
        self.mutate(target, op="approve_invitation", invitation=invitation["invitation"])

    def send_rpc(self, client, method, params=None):
        self.request_id += 1
        port = client["registry_port"]
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=20)
        try:
            connection.request("POST", "/mcp", json.dumps({
                "jsonrpc": "2.0", "id": self.request_id, "method": method,
                "params": params or {},
            }), headers={"Host": f"wispnet.localhost:{port}",
                         "Content-Type": "application/json",
                         "Accept": "application/json, text/event-stream"})
            return connection
        except BaseException:
            connection.close()
            raise

    def rpc(self, client, method, params=None):
        return self.receive_rpc(self.send_rpc(client, method, params))

    def receive_rpc(self, connection):
        try:
            response = connection.getresponse()
            self.assertEqual(response.status, 200)
            return json.loads(response.read())
        finally:
            connection.close()

    def run_command(self, client, host, command, **options):
        return self.rpc(client, "tools/call", {
            "name": "wispnet_run_shell_command",
            "arguments": dict(host=host, command=command, **options),
        })

    def success(self, response):
        self.assertNotIn("error", response)
        self.assertFalse(response["result"]["isError"], response)
        return response["result"]["structuredContent"]

    def denied(self, response):
        self.assertTrue("error" in response or response["result"]["isError"], response)

    def start_command_with_child(self, client, target, label, child_delay=2):
        pidfile = self.root / (label + "-shell.pid")
        marker = self.root / (label + "-child-survived")
        command = ("printf '%s\\n' \"$$\" > " + shlex.quote(str(pidfile))
                   + f"; (sleep {child_delay}; printf leaked > " + shlex.quote(str(marker)) + ") & wait")
        connection = self.send_rpc(client, "tools/call", {
            "name": "wispnet_run_shell_command",
            "arguments": dict(host=target["peer"], command=command),
        })
        deadline = time.monotonic() + 5
        try:
            while True:
                try:
                    pid = int(pidfile.read_text())
                    return connection, pid, marker
                except (FileNotFoundError, ValueError):
                    self.assertLess(time.monotonic(), deadline, "Command did not start")
                    time.sleep(.03)
        except BaseException:
            connection.close()
            raise

    def assert_command_and_children_stopped(self, pid, marker, cancelled_at):
        while True:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                break
            self.assertLess(time.monotonic() - cancelled_at, 3,
                            "Cancelled MCP request left its shell running")
            time.sleep(.03)
        time.sleep(max(0, cancelled_at + 2.3 - time.monotonic()))
        self.assertFalse(marker.exists(), "Cancelled MCP request left a running background child")

    def shell_input(self, api, session, text):
        api.request(op="shell_input", session=session,
                    data_b64=base64.b64encode(text.encode()).decode())

    def shell_expect(self, api, session, marker, exit_code=None):
        output = bytearray()
        exited = None
        deadline = time.monotonic() + 8
        while marker.encode() not in output or exit_code is not None and exited is None:
            poll = api.request(op="shell_poll", session=session)
            for event in poll["events"]:
                if event["type"] in ["output", "scrollback"]:
                    output.extend(base64.b64decode(event["data_b64"]))
                elif event["type"] == "exit":
                    exited = event["code"]
                elif event["type"] == "error":
                    self.fail(f"Interactive shell failed while testing Exec: {event}")
            self.assertLess(time.monotonic(), deadline, f"Interactive shell stalled: {bytes(output)!r}")
            time.sleep(.03)
        if exit_code is not None:
            self.assertEqual(exited, exit_code)

    def test_remote_command_permissions_outputs_limits_and_revocation(self):
        client = self.host("mcp-client")
        target = self.host("j-desktop")
        marker = self.root / "must-not-execute"
        forbidden_command = "printf forbidden > " + shlex.quote(str(marker))

        # A target must be paired and must explicitly grant Shell access.
        self.denied(self.run_command(client, target["peer"], forbidden_command))
        self.pair(client, target)
        self.denied(self.run_command(client, target["peer"], forbidden_command))
        self.assertFalse(marker.exists())
        self.mutate(target, op="grant_service", service="shell", peer=client["peer"])
        before = target["api"].request(op="status")["host"]

        tools = self.rpc(client, "tools/list")["result"]["tools"]
        schema = next(t["inputSchema"] for t in tools if t["name"] == "wispnet_run_shell_command")
        self.assertEqual(set(schema["required"]), {"host", "command"})
        self.assertEqual(set(schema["properties"]), {
            "host", "command", "cwd", "timeout_seconds", "max_output_bytes"})
        self.assertFalse(schema["additionalProperties"])

        # Native Exec has independent streams, no PTY, and returns nonzero exits as data.
        output = self.success(self.run_command(client, target["peer"],
            "printf 'standard output\\n'; printf 'standard error\\n' >&2; exit 7"))
        self.assertEqual(output["host"], target["peer"])
        self.assertEqual(output["stdout"], "standard output\n")
        self.assertEqual(output["stderr"], "standard error\n")
        self.assertEqual(output["exit_code"], 7)
        self.assertFalse(output["timed_out"])
        self.assertFalse(output["output_truncated"])
        output = self.success(self.run_command(client, target["peer"],
                                               r"printf '\377'; printf '\376' >&2"))
        self.assertEqual(output["stdout"], "\ufffd")
        self.assertEqual(output["stderr"], "\ufffd")

        # Closing an Exec stream must preserve an already-open interactive shell.
        incarnation = next(s["incarnation"] for s in before["services"] if s["id"] == "shell")
        interactive = client["api"].request(op="shell_open", host=target["peer"],
            service="shell", incarnation=incarnation, session="mcp-compat")["session"]
        try:
            self.shell_input(client["api"], interactive,
                "WISP_SESSION_VALUE=kept; printf '%s%s\\n' WISP_ INTERACTIVE_READY\r")
            self.shell_expect(client["api"], interactive, "WISP_INTERACTIVE_READY")
            output = self.success(self.run_command(client, target["peer"],
                "printf '%s' \"${WISP_SESSION_VALUE-unset}\""))
            self.assertEqual(output["stdout"], "unset")
            self.shell_input(client["api"], interactive,
                "printf 'WISP_RETAINED_%s\\n' \"$WISP_SESSION_VALUE\"; exit 0\r")
            self.shell_expect(client["api"], interactive, "WISP_RETAINED_kept", exit_code=0)
        finally:
            client["api"].request(op="shell_close", session=interactive)

        # Readable names resolve to the same pinned identity. No PTY session state leaks.
        output = self.success(self.run_command(client, "j-desktop",
            "pwd; printf '%s\\n' \"$HOME\"; if read -r line; then exit 9; fi; "
            "if [ -t 0 ] || [ -t 1 ] || [ -t 2 ]; then exit 8; fi; WISP_EXEC_STATE=private"))
        self.assertEqual(output["stdout"], f'{target["home"]}\n{target["home"]}\n')
        self.assertEqual(output["exit_code"], 0)
        work = self.root / "working directory 'quoted'"
        work.mkdir()
        command = "printf '%s\\n' \"first line\" 'quote: \" and $ literal'\npwd\nprintf '%s' \"${WISP_EXEC_STATE-unset}\""
        output = self.success(self.run_command(client, target["peer"], command, cwd=str(work)))
        self.assertEqual(output["stdout"], f'first line\nquote: " and $ literal\n{work}\nunset')

        output = self.success(self.run_command(client, target["peer"],
            "i=0; while [ \"$i\" -lt 1000 ]; do printf 0123456789; i=$((i+1)); done", max_output_bytes=64))
        self.assertTrue(output["output_truncated"])
        self.assertEqual(len(output["stdout"].encode()) + len(output["stderr"].encode()), 64)
        self.assertEqual(output["exit_code"], 0)

        # A timeout also kills background children; they cannot write after the reply.
        leaked = self.root / "timed-out-child-survived"
        pidfile = self.root / "timed-out-shell.pid"
        command = ("printf '%s\\n' \"$$\" > " + shlex.quote(str(pidfile))
                   + "; (sleep 2; printf leaked > " + shlex.quote(str(leaked)) + ") & wait")
        started = time.monotonic()
        output = self.success(self.run_command(client, target["peer"], command, timeout_seconds=1))
        self.assertTrue(output["timed_out"])
        self.assertLess(time.monotonic() - started, 8)
        self.assertTrue(pidfile.exists())
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pidfile.read_text()), 0)
        time.sleep(1.3)
        self.assertFalse(leaked.exists(), "Timed-out command left a running background child")

        # Closing the MCP HTTP request must cancel the native stream and its process group.
        connection, pid, disconnected_marker = self.start_command_with_child(client, target, "disconnected")
        connection.close()
        disconnected_at = time.monotonic()
        self.assert_command_and_children_stopped(pid, disconnected_marker, disconnected_at)

        for options in [dict(cwd="relative"), dict(timeout_seconds=0),
                        dict(timeout_seconds=301), dict(max_output_bytes=0),
                        dict(max_output_bytes=1048577), dict(unexpected=True)]:
            self.denied(self.run_command(client, target["peer"], forbidden_command, **options))
        self.denied(self.run_command(client, "unknown-host", forbidden_command))
        self.assertFalse(marker.exists())
        self.assertEqual(target["api"].request(op="status")["host"], before)

        # Four commands fill the execution budget while owner controls stay usable.
        running = []
        overflow_marker = self.root / "capacity-overflow-executed"
        try:
            for index in range(4):
                running.append(self.start_command_with_child(
                    client, target, f"capacity-{index}", child_delay=30))
            overflow = self.send_rpc(client, "tools/call", {
                "name": "wispnet_run_shell_command",
                "arguments": dict(host=target["peer"], command=
                    "printf overflow > " + shlex.quote(str(overflow_marker))),
            })
            try:
                response = overflow.getresponse()
                self.assertEqual(response.status, 503)
                response.read()
            finally:
                overflow.close()
            self.assertFalse(overflow_marker.exists())
            self.success(self.rpc(client, "tools/call", {
                "name": "wispnet_status", "arguments": {},
            }))
        finally:
            for connection, _, _ in running:
                connection.close()
        deadline = time.monotonic() + 3
        for _, pid, _ in running:
            while True:
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    break
                self.assertLess(time.monotonic(), deadline, "Capacity check left a shell running")
                time.sleep(.03)
        self.assertFalse(overflow_marker.exists())

        # Remote grant revocation cancels active work as well as denying new calls.
        connection, pid, revoked_marker = self.start_command_with_child(client, target, "revoked")
        revoked_at = time.monotonic()
        try:
            self.mutate(target, op="revoke_service", service="shell", peer=client["peer"])
            self.denied(self.receive_rpc(connection))
        finally:
            connection.close()
        self.assertLess(time.monotonic() - revoked_at, 5)
        self.assert_command_and_children_stopped(pid, revoked_marker, revoked_at)
        after = target["api"].request(op="status")["host"]
        self.denied(self.run_command(client, "j-desktop", forbidden_command))
        self.assertFalse(marker.exists())
        self.assertEqual(after["clients"], before["clients"])
        self.assertEqual(target["api"].request(op="status")["host"], after)

        # Forgetting the host locally must cancel work without changing remote grants.
        self.mutate(target, op="grant_service", service="shell", peer=client["peer"])
        remote_before = target["api"].request(op="status")["host"]
        connection, pid, unpaired_marker = self.start_command_with_child(client, target, "unpaired")
        unpaired_at = time.monotonic()
        try:
            client["api"].request(op="unpair", host=target["peer"])
            self.denied(self.receive_rpc(connection))
        finally:
            connection.close()
        self.assertLess(time.monotonic() - unpaired_at, 5)
        self.assert_command_and_children_stopped(pid, unpaired_marker, unpaired_at)
        self.denied(self.run_command(client, target["peer"], forbidden_command))
        self.assertFalse(marker.exists())
        self.assertEqual(target["api"].request(op="status")["host"], remote_before)


if __name__ == "__main__":
    unittest.main()
