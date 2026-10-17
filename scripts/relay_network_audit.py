#!/usr/bin/env python3
"""Linux relay fixtures in a private network namespace with syscall auditing.

Requires unprivileged user/network namespaces, iproute2, nftables and strace.
Never changes the host's firewall, interfaces, routes or resolver configuration.
"""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


def complete_calls(trace):
    """Reassemble interleaved strace calls before inspecting addresses/results."""
    pending = {}
    for line in trace.splitlines():
        match = re.match(r"(\d+)\s+(.*)", line)
        if not match:
            raise RuntimeError("unrecognized syscall trace format")
        pid, call = match.groups()
        if call.endswith("<unfinished ...>"):
            if pid in pending:
                raise RuntimeError("overlapping incomplete syscall trace")
            pending[pid] = call.removesuffix("<unfinished ...>")
        elif resumed := re.match(r"<\.\.\. (\w+) resumed>(.*)", call):
            if pid not in pending:
                raise RuntimeError("unmatched resumed syscall trace")
            yield pending.pop(pid) + resumed.group(2)
        else:
            yield call
    if pending:
        raise RuntimeError("incomplete syscall trace")


def audit(trace):
    """Reject even failed attempts to contact non-loopback Internet addresses."""
    destinations = set()
    calls = 0
    route_probes = 0
    for line in complete_calls(trace):
        if not re.search(r"\b(connect|sendto|sendmsg|sendmmsg)\(", line):
            continue
        calls += 1
        addresses = re.findall(r'sin_addr=inet_addr\("([^"\n]+)"\)', line)
        addresses += re.findall(r'inet_pton\(AF_INET6, "([^"\n]+)", &sin6_addr\)', line)
        for address in addresses:
            parsed = ipaddress.ip_address(address)
            if isinstance(parsed, ipaddress.IPv6Address) and parsed.ipv4_mapped:
                parsed = parsed.ipv4_mapped
            if not parsed.is_loopback:
                # netdev uses UDP connect (no packets) solely to ask the kernel
                # which interface would route these two fixed destinations.
                # Only accept its precise, failed no-route probe, never sends,
                # TCP connects, arbitrary private addresses, or successful calls.
                route_probe = (
                    str(parsed) in {"10.254.254.254", "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"}
                    and re.search(r"\bconnect\(\d+<UDP(?:v6)?:", line)
                    and re.search(r"sin(?:6)?_port=htons\(1\)", line)
                    and "= -1 ENETUNREACH" in line
                )
                if route_probe:
                    route_probes += 1
                    continue
                raise RuntimeError(f"unexpected Internet destination: {parsed}")
            destinations.add(str(parsed))
    if not calls or not destinations:
        raise RuntimeError("empty network trace is not evidence")
    return destinations, route_probes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--strace", default=shutil.which("strace"))
    args = parser.parse_args()
    for name in ("unshare", "ip", "nft"):
        if not shutil.which(name):
            parser.error(f"missing {name}; isolated audit cannot run")
    if not args.strace or not Path(args.strace).is_file():
        parser.error("strace is required (or pass --strace /absolute/path/strace)")
    root = Path(__file__).resolve().parent.parent
    build = subprocess.run(
        ["cargo", "test", "--workspace", "--locked", "--no-run", "--message-format=json"],
        cwd=root, check=True, text=True, stdout=subprocess.PIPE,
    )
    wanted = {"wipsnet_agent": "tunnel::relay_tests", "host_control": "", "relay_cli": "", "pairing": ""}
    sources = {
        "wipsnet_agent": root / "crates/agent/src/lib.rs",
        "host_control": root / "crates/agent/tests/host_control.rs",
        "relay_cli": root / "apps/node/cli/tests/relay_cli.rs",
        "pairing": root / "crates/agent/tests/pairing.rs",
    }
    binaries = {}
    for line in build.stdout.splitlines():
        entry = json.loads(line)
        if entry.get("reason") == "compiler-artifact" and entry["profile"].get("test"):
            name = entry["target"]["name"]
            if (name in wanted and entry.get("executable")
                    and Path(entry["target"]["src_path"]) == sources[name]):
                binaries[name] = entry["executable"]
    if binaries.keys() != wanted.keys():
        raise RuntimeError("missing expected relay test executables")
    # All network mutations happen after unshare. Only loopback destinations are
    # permitted; a syscall trace also detects attempts that the firewall rejects.
    setup = """ip link set lo up
nft -f - <<'RULES'
table inet wipsnet_test {
    chain output {
        type filter hook output priority 0; policy drop;
        oifname "lo" ip daddr 127.0.0.0/8 accept
        oifname "lo" ip6 daddr ::1 accept
    }
}
RULES
if [ "$WIPSNET_AUDIT_DROP_UDP" = 1 ]; then
    nft insert rule inet wipsnet_test output meta l4proto udp counter drop
fi
"$@"
nft -j list ruleset > "$WIPSNET_AUDIT_FIREWALL_REPORT"
"""
    with tempfile.TemporaryDirectory(prefix="wipsnet-network-audit-") as directory:
        jobs = [(name, executable, wanted[name], False) for name, executable in binaries.items()]
        jobs.append(("udp_blocked", binaries["host_control"], "udp_enabled_hosts_fall_back_to_relay_with_stale_hint", True))
        for name, executable, test_filter, drop_udp in jobs:
            trace = Path(directory) / f"{name}.trace"
            command = ["unshare", "--user", "--map-root-user", "--net", "sh", "-eu", "-c", setup,
                       "wipsnet-isolated-test", str(Path(args.strace).resolve()),
                       "-f", "-qq", "-yy", "-s", "0", "-e", "trace=network", "-o", str(trace),
                       executable]
            if test_filter:
                command.append(test_filter)
            report = Path(directory) / f"{name}.firewall.json"
            environment = dict(os.environ, WIPSNET_AUDIT_DROP_UDP=str(int(drop_udp)), WIPSNET_AUDIT_FIREWALL_REPORT=str(report))
            subprocess.run(command, cwd=root, env=environment, check=True, timeout=90)
            if drop_udp:
                rules = json.loads(report.read_text())["nftables"]
                dropped = sum(expr.get("counter", {}).get("packets", 0)
                              for item in rules if "rule" in item
                              for expr in item["rule"]["expr"])
                if not dropped:
                    raise RuntimeError("forced UDP test did not exercise the drop rule")
                print(f"{name}: kernel dropped {dropped} UDP packets; TLS relay catalog/service still passed", flush=True)
            destinations, probes = audit(trace.read_text())
            print(f"{name}: firewall isolation passed; audited destinations: {', '.join(sorted(destinations))}; {probes} failed no-packet route probes", flush=True)
    print("Isolated relay/DNS/native CLI audit passed; no non-loopback TCP connects or addressed packet sends; only documented failed UDP route probes.")


if __name__ == "__main__":
    main()
