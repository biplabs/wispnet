#!/usr/bin/env python3
"""Real CLI/PTY shell checks over two isolated Wispnet nodes.

Uses private temporary identities, policies, HOME directories, and loopback
discovery hints. No installed node, user grants, or shell profiles are touched.
"""

import errno
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import pty
import select
import signal
import socket
import struct
import subprocess
import tempfile
import termios
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/wispnet"
spec = importlib.util.spec_from_file_location(
    "shell_node_api", ROOT / "apps/node/desktop/wispnet_desktop/api.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Terminal:
    """Run the real CLI with a controlling terminal and retain byte output."""

    def __init__(self, command, environment):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.execve(BINARY, [str(arg) for arg in command], environment)
        self.original = termios.tcgetattr(self.fd)
        self.output = bytearray()
        self.exited = False
        self.resize(80, 24)

    def resize(self, cols, rows):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        os.kill(self.pid, signal.SIGWINCH)

    def read(self, seconds=0.05):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if not select.select([self.fd], [], [], max(0, deadline - time.monotonic()))[0]:
                return
            try:
                data = os.read(self.fd, 65536)
            except OSError as error:
                if error.errno == errno.EIO:
                    return
                raise
            if not data:
                return
            self.output.extend(data)

    def expect(self, needle, after=0, timeout=12):
        needle = needle.encode() if isinstance(needle, str) else needle
        deadline = time.monotonic() + timeout
        while needle not in self.output[after:]:
            self.read()
            if time.monotonic() >= deadline:
                raise AssertionError(f"Terminal never showed {needle!r}: {bytes(self.output)!r}")

    def send(self, data):
        os.write(self.fd, data.encode() if isinstance(data, str) else data)

    def wait(self, expected, timeout=12):
        deadline = time.monotonic() + timeout
        while True:
            self.read()
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.exited = True
                code = os.waitstatus_to_exitcode(status)
                if code != expected:
                    raise AssertionError(f"Shell CLI exited {code}, expected {expected}: {bytes(self.output)!r}")
                flags = termios.ICANON | termios.ECHO
                if termios.tcgetattr(self.fd)[3] & flags != self.original[3] & flags:
                    raise AssertionError("Shell CLI did not restore terminal settings")
                return
            if time.monotonic() >= deadline:
                raise AssertionError(f"Shell CLI did not exit: {bytes(self.output)!r}")

    def close(self):
        if not self.exited:
            os.kill(self.pid, signal.SIGTERM)
            try:
                self.wait(0, timeout=4)
            except AssertionError:
                if not self.exited:
                    os.kill(self.pid, signal.SIGKILL)
                    os.waitpid(self.pid, 0)
                    self.exited = True
        os.close(self.fd)


class ShellSmoke(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wispnet-shell-")
        self.root = Path(self.temporary.name)
        home = self.root / "home"
        home.mkdir(mode=0o700)
        self.environment = dict(os.environ, HOME=str(home), SHELL="/bin/sh", ENV="",
                                TERM="xterm-256color", NO_COLOR="1")
        self.processes = []
        self.terminals = []

    def tearDown(self):
        try:
            for terminal in self.terminals:
                terminal.close()
        finally:
            for process in self.processes:
                process.terminate()
                try:
                    process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            self.temporary.cleanup()

    def cli(self, config, *args, input=None, success=True):
        result = subprocess.run([BINARY, *args, "--config", config], input=input,
                                capture_output=True, text=True, timeout=20,
                                env=self.environment)
        self.assertEqual(result.returncode == 0, success, result.stderr)
        return result

    def host(self, name):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.bind(("127.0.0.1", 0))
            port = udp.getsockname()[1]
        config, state = self.root / (name + ".toml"), self.root / name
        config.write_text(
            f'schema_version=2\ndevice_name="{name}"\nstate_dir="{state}"\nlog_level="info"\n'
            f'[transport]\nbind_addr="127.0.0.1:{port}"\nrelay_url="https://relay.example"\n'
        )
        # The low-level management parser requires --config immediately after op.
        initialized = subprocess.run([BINARY, "host-init", "--config", config],
                                     capture_output=True, text=True, check=True,
                                     env=self.environment, timeout=15)
        peer = initialized.stdout.split("Endpoint ID: ")[1].strip()
        daemon = subprocess.Popen([BINARY, "agent", "--config", config],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                  env=self.environment)
        self.processes.append(daemon)
        deadline = time.monotonic() + 12
        api = module.NodeAPI(config)
        while True:
            self.assertIsNone(daemon.poll(), "Fixture node exited before its API was ready")
            try:
                api.request(op="status")
                break
            except (FileNotFoundError, ConnectionRefusedError):
                self.assertLess(time.monotonic(), deadline, "Fixture node did not start")
                time.sleep(0.03)
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as registry:
            registry.bind(("127.0.0.1", 0))
            registry_port = registry.getsockname()[1]
        api.request(op="configure_access", settings=dict(
            registry_port=registry_port, allowed_origins=[], service_ports=[]))
        api.request(op="enable")
        return config, api, peer, port

    def mutate(self, api, **operation):
        operation["expected_revision"] = api.request(op="status")["host"]["revision"]
        return api.request(op="mutate", operation=operation)

    def terminal(self, config, host, incarnation, session="main"):
        terminal = Terminal([BINARY, "shell", "--host", host, "--service", "shell",
                             "--incarnation", incarnation, "--session", session,
                             "--config", config], self.environment)
        self.terminals.append(terminal)
        terminal.expect("Ctrl+] detaches")
        return terminal

    def test_bulk_input_and_output_make_progress_together(self):
        config, _, _, _ = self.host("shell-bulk")
        # More than the host's 1 MiB history budget: draining stdin before
        # polling output deadlocks or overruns, even though each RPC is bounded.
        # The expected marker is absent from the echoed command itself.
        count = 50_000
        payload = "printf '%s%s\\n' REVIEW_ PASTE\n" * count + "exit 0\n"
        result = self.cli(config, "shell", "localhost", "--session", "bulk", input=payload)
        self.assertEqual(result.stdout.count("REVIEW_PASTE"), count)

    def test_permissions_persistent_session_resize_exit_and_revocation(self):
        client, api, client_id, _ = self.host("shell-client")
        target, target_api, target_id, target_port = self.host("shell-target")
        builtin = next(s for s in target_api.request(op="status")["host"]["services"] if s["id"] == "shell")
        self.assertEqual(builtin["kind"], "shell")
        self.assertEqual(builtin["allow"], [])
        for operation in [dict(op="service", id="shell", kind="http", backend="127.0.0.1:8000", allow=[]),
                          dict(op="remove", id="shell")]:
            with self.assertRaises(ValueError):
                self.mutate(target_api, **operation)
        invitation_path = self.root / "invitation.json"
        subprocess.run([BINARY, "host-invite", "--config", target, "--output", invitation_path],
                       capture_output=True, check=True, env=self.environment, timeout=15)
        invitation = json.loads(invitation_path.read_text())
        invitation["direct_hint"] = f"127.0.0.1:{target_port}"
        api.request(op="pair", host=target_id, invitation=json.dumps(invitation))
        self.mutate(target_api, op="approve_invitation", invitation=invitation["invitation"])

        # Pairing alone must not enable remote command execution.
        self.cli(client, "shell", "--host", target_id, success=False)
        self.assertEqual(api.request(op="catalog", host=target_id)["services"], [])
        self.cli(client, "shell", "--host", target_id, success=False)
        self.mutate(target_api, op="shell", allow=[client_id])
        catalog = api.request(op="catalog", host=target_id)
        self.assertEqual(catalog["services"][0]["kind"], "shell")
        incarnation = catalog["services"][0]["incarnation"]
        failure = self.cli(client, "shell", "--host", target_id, "--incarnation", "f" * 64,
                           success=False)
        self.assertIn("changed", failure.stderr)

        terminal = self.terminal(client, target_id, incarnation)
        terminal.send("WISP_SMOKE_VALUE=kept; printf '%s%s\\n' WISP_ READY\r")
        terminal.expect("WISP_READY")
        terminal.resize(93, 31)
        time.sleep(0.15)  # Let the CLI observe and transmit the changed dimensions.
        terminal.send("printf 'WISP_SIZE_'; stty size\r")
        terminal.expect("WISP_SIZE_31 93")
        terminal.send(b"\x1d")
        terminal.wait(0)

        resumed = self.terminal(client, target_id, incarnation)
        resumed.send("printf 'WISP_RESUME_%s\\n' \"$WISP_SMOKE_VALUE\"; exit 7\r")
        resumed.expect("WISP_RESUME_kept")
        resumed.wait(7)

        # Owner-local desktop/CLI access needs no self-grant or second identity.
        local = self.cli(target, "shell", "localhost", "--session", "owner",
                         input="printf '%s%s\\n' WISP_ LOCAL; exit 0\n")
        self.assertIn("WISP_LOCAL", local.stdout)

        active = self.terminal(client, target_id, incarnation, session="revoked")
        active.send("printf '%s%s\\n' WISP_ ACTIVE\r")
        active.expect("WISP_ACTIVE")
        self.mutate(target_api, op="shell", allow=[])
        active.wait(1)
        self.cli(client, "shell", "--host", target_id, success=False)

        # Forgetting a host locally must also close an already-open outgoing
        # terminal, without relying on the remote owner learning the removal.
        self.mutate(target_api, op="shell", allow=[client_id])
        removed = self.terminal(client, target_id, incarnation, session="unpaired")
        removed.send("printf '%s%s\\n' WISP_ UNPAIR_READY\r")
        removed.expect("WISP_UNPAIR_READY")
        api.request(op="unpair", host=target_id)
        self.assertNotIn(target_id, api.request(op="status")["peer_pins"])
        self.assertTrue(any(c["peer"] == client_id for c in target_api.request(op="status")["host"]["clients"]))
        removed.wait(1)
        self.cli(client, "shell", "--host", target_id, success=False)


if __name__ == "__main__":
    unittest.main()
