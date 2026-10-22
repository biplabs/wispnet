#!/usr/bin/env python3
"""Root-only socket-verified firewall reconciler; no RPC, shell, or arbitrary rules."""
import fcntl
import ipaddress
import json
import os
from pathlib import Path
import re
import shlex
import signal
import stat
import subprocess
import time

CONFIG = Path('/etc/wispnet-firewall.json')
STATUS = Path('/run/wispnet-firewall/status.json')
CHAIN = 'WISPNET-UDP'
MAX_REPORT = 4096


def read_json(path, owner, private=False):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        m = os.fstat(fd)
        if not stat.S_ISREG(m.st_mode) or m.st_uid != owner or m.st_mode & (0o077 if private else 0o022):
            raise ValueError('Unsafe report/config permissions')
        limit = MAX_REPORT if private else 16384
        data = os.read(fd, limit + 1)
        if len(data) > limit:
            raise ValueError('Oversized report/config')
        return json.loads(data)
    finally:
        os.close(fd)


def configured_uids():
    value = read_json(CONFIG, 0)
    if set(value) != {'version', 'uids'} or value['version'] != 1:
        raise ValueError('Invalid helper configuration')
    uids = value['uids']
    if not isinstance(uids, list) or len(uids) > 32 or any(type(u) is not int or not 0 < u < 2**32 for u in uids):
        raise ValueError('Invalid allowed users')
    return set(uids)


def verified_ports(uid, proc=Path('/proc'), runtime=Path('/run/user')):
    """Return only reported ports actually held by this user's live host service."""
    directory = runtime / str(uid)
    m = directory.lstat()
    if not stat.S_ISDIR(m.st_mode) or m.st_uid != uid or m.st_mode & 0o077:
        raise ValueError('Unsafe runtime directory')
    report = read_json(directory / 'wispnet-firewall.json', uid, private=True)
    if set(report) != {'version', 'pid', 'start_time', 'ports'} or report['version'] != 1:
        raise ValueError('Invalid socket report')
    pid = report['pid']
    if type(pid) is not int or not 1 < pid < 2**31:
        raise ValueError('Invalid process')
    process = proc / str(pid)
    uid_line = next(line for line in (process / 'status').read_text().splitlines() if line.startswith('Uid:'))
    if [int(u) for u in uid_line.split()[1:]] != [uid] * 4:
        raise ValueError('Process user mismatch')
    expected = f'0::/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/wispnet-host.service'
    if (process / 'cgroup').read_text().strip() != expected:
        raise ValueError('Not the Wispnet host unit')
    if (process / 'stat').read_text().rsplit(')', 1)[1].split()[19] != report['start_time']:
        raise ValueError('Reused process ID')
    if os.stat(process / 'ns/net') != os.stat(proc / 'self/ns/net'):
        # Compare inode/device only; other stat metadata can differ between proc links.
        a, b = os.stat(process / 'ns/net'), os.stat(proc / 'self/ns/net')
        if (a.st_dev, a.st_ino) != (b.st_dev, b.st_ino):
            raise ValueError('Different network namespace')
    entries = report['ports']
    if not isinstance(entries, list) or len(entries) > 2:
        raise ValueError('Too many sockets')
    requested = set()
    for item in entries:
        if not isinstance(item, dict) or set(item) != {'family', 'port'} or item['family'] not in ('ipv4', 'ipv6') or type(item['port']) is not int or not 1024 <= item['port'] <= 65535:
            raise ValueError('Invalid UDP socket')
        requested.add((uid, item['family'], item['port']))
    inodes = set()
    for fd in (process / 'fd').iterdir():
        try:
            target = os.readlink(fd)
        except FileNotFoundError:
            continue
        if re.fullmatch(r'socket:\[\d+\]', target):
            inodes.add(target[8:-1])
    owned = set()
    for family, table in [('ipv4', 'udp'), ('ipv6', 'udp6')]:
        for line in (process / 'net' / table).read_text().splitlines()[1:]:
            columns = line.split()
            address, port = columns[1].split(':')
            if columns[9] not in inodes or int(columns[7]) != uid:
                continue
            raw = bytes.fromhex(address)
            raw = b''.join(raw[i:i+4][::-1] for i in range(0, len(raw), 4))
            if not ipaddress.ip_address(raw).is_loopback:
                owned.add((uid, family, int(port, 16)))
    if not requested <= owned:
        raise ValueError('Reported port is not an owned UDP socket')
    # Recheck process generation after walking descriptors to close PID reuse races.
    if (process / 'stat').read_text().rsplit(')', 1)[1].split()[19] != report['start_time']:
        raise ValueError('Process changed during verification')
    return requested


class Firewall:
    def __init__(self):
        self.backends = {}
        for family, name in [('ipv4', 'iptables'), ('ipv6', 'ip6tables')]:
            for directory in ['/usr/sbin', '/usr/bin', '/sbin']:
                path = Path(directory) / name
                if path.exists():
                    m = path.stat()
                    if m.st_uid != 0 or m.st_mode & 0o022:
                        raise ValueError('Unsafe firewall executable')
                    self.backends[family] = str(path)
                    break
        if 'ipv4' not in self.backends:
            raise ValueError('iptables is required')

    def call(self, family, args, check=True):
        result = subprocess.run([self.backends[family], '-w', '5', *args], text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10,
                                env={'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LANG': 'C'})
        if check and result.returncode:
            raise RuntimeError('Firewall command failed: ' + result.stderr.strip())
        return result

    @staticmethod
    def rule(uid, port):
        return ['-p', 'udp', '-m', 'udp', '--dport', str(port), '-m', 'comment',
                '--comment', f'wispnet-uid-{uid}', '-j', 'ACCEPT']

    def reconcile(self, desired, cleanup=False):
        for family in ('ipv4', 'ipv6'):
            if family not in self.backends:
                if any(f == family for _, f, _ in desired):
                    raise RuntimeError('Missing IPv6 firewall backend')
                continue
            current = self.call(family, ['-S', CHAIN], check=False)
            if current.returncode:
                if cleanup:
                    continue
                self.call(family, ['-N', CHAIN])
                current_rules = []
            else:
                current_rules = []
                for line in current.stdout.splitlines():
                    words = shlex.split(line)
                    if words == ['-N', CHAIN]:
                        continue
                    # Refuse to flush or modify a chain containing non-Wispnet rules.
                    if len(words) != 14 or words[:2] != ['-A', CHAIN] or not re.fullmatch(r'wispnet-uid-[1-9]\d*', words[11]):
                        raise ValueError('Foreign rule in Wispnet chain')
                    uid, port = int(words[11].removeprefix('wispnet-uid-')), int(words[7])
                    if words[2:] != self.rule(uid, port) or not 1024 <= port <= 65535:
                        raise ValueError('Foreign rule in Wispnet chain')
                    current_rules.append((uid, family, port))
            wanted = set() if cleanup else {p for p in desired if p[1] == family}
            for uid, _, port in sorted(wanted - set(current_rules)):
                self.call(family, ['-A', CHAIN, *self.rule(uid, port)])
            for uid, _, port in sorted(set(current_rules) - wanted):
                self.call(family, ['-D', CHAIN, *self.rule(uid, port)])
            jump = ['-j', CHAIN]
            # Keep our one jump before UFW's INPUT chains, including after a reload.
            rules = self.call(family, ['-S', 'INPUT']).stdout.splitlines()
            first = next((shlex.split(x) for x in rules if x.startswith('-A INPUT ')), None)
            if cleanup or first != ['-A', 'INPUT', *jump]:
                while self.call(family, ['-C', 'INPUT', *jump], check=False).returncode == 0:
                    self.call(family, ['-D', 'INPUT', *jump])
                if not cleanup:
                    self.call(family, ['-I', 'INPUT', '1', *jump])
            if cleanup:
                self.call(family, ['-X', CHAIN])


def publish(healthy, ports, uids, error=None):
    value = {'version': 1, 'healthy': healthy, 'uids': sorted(uids),
             'ports': [{'uid': u, 'family': f, 'port': p} for u, f, p in sorted(ports)],
             'updated': time.time(), 'error': error}
    tmp = STATUS.with_suffix('.tmp')
    tmp.write_text(json.dumps(value))
    tmp.chmod(0o644)
    tmp.replace(STATUS)


def main():
    if os.geteuid() != 0:
        raise SystemExit('The firewall helper must run as root')
    STATUS.parent.mkdir(mode=0o755, exist_ok=True)
    lock = open(STATUS.parent / 'lock', 'w')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    firewall = Firewall()
    stopping = False
    def stop(*_):
        nonlocal stopping
        stopping = True
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        while not stopping:
            desired, uids = set(), set()
            try:
                uids = configured_uids()
                for uid in uids:
                    try:
                        desired |= verified_ports(uid)
                    except (OSError, ValueError, KeyError, IndexError, StopIteration):
                        # Missing, stopped or invalid nodes lose their allow rule.
                        continue
                firewall.reconcile(desired)
                publish(True, desired, uids)
            except Exception as error:
                # Do not report success if verification or a backend operation failed.
                try:
                    firewall.reconcile(set())
                except Exception:
                    pass
                publish(False, set(), uids, str(error))
                print(str(error), flush=True)
            time.sleep(1)
    finally:
        try:
            firewall.reconcile(set(), cleanup=True)
        finally:
            publish(False, set(), set(), 'helper stopped')


if __name__ == '__main__':
    main()
