"""Prevent the network audit from accepting misleading or incomplete traces."""
import unittest
from relay_network_audit import audit

LOCAL = '123 connect(7<TCP:[123]>, {sa_family=AF_INET, sin_port=htons(443), sin_addr=inet_addr("127.0.0.1")}, 16) = -1 EINPROGRESS\n'
PROBE = '123 connect(8<UDP:[0.0.0.0:123]>, {sa_family=AF_INET, sin_port=htons(1), sin_addr=inet_addr("10.254.254.254")}, 16) = -1 ENETUNREACH\n'


class AuditTests(unittest.TestCase):
    def test_loopback_and_failed_no_packet_probe(self):
        self.assertEqual(audit(LOCAL + PROBE), ({"127.0.0.1"}, 1))

    def test_empty_or_probe_only_is_not_evidence(self):
        for trace in ("", PROBE):
            with self.assertRaisesRegex(RuntimeError, "empty"):
                audit(trace)

    def test_failed_public_connect_is_still_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "unexpected Internet destination"):
            audit(LOCAL + LOCAL.replace("127.0.0.1", "8.8.8.8"))

    def test_probe_exception_does_not_allow_packets_tcp_or_success(self):
        for line in (PROBE.replace("connect(", "sendto("), PROBE.replace("<UDP:", "<TCP:"),
                     PROBE.replace("= -1 ENETUNREACH", "= 0"), PROBE.replace("htons(1)", "htons(53)")):
            with self.assertRaisesRegex(RuntimeError, "unexpected Internet destination"):
                audit(LOCAL + line)

    def test_ipv6_and_mapped_ipv4_are_checked(self):
        for address in ("::1", "::ffff:127.0.0.2"):
            line = f'123 connect(7<TCPv6:[123]>, {{sa_family=AF_INET6, inet_pton(AF_INET6, "{address}", &sin6_addr)}}, 28) = 0'
            self.assertTrue(audit(LOCAL + line)[0])
        with self.assertRaisesRegex(RuntimeError, "unexpected Internet destination"):
            audit(LOCAL + '123 sendmsg(8, {inet_pton(AF_INET6, "2606:4700:4700::1111", &sin6_addr)}, 0) = -1 ENETUNREACH')

    def test_sendmmsg_and_unfinished_calls_are_checked(self):
        with self.assertRaisesRegex(RuntimeError, "unexpected Internet destination"):
            audit(LOCAL + '123 sendmmsg(8, [{msg_hdr={msg_name={sin_addr=inet_addr("192.0.2.1")}}}], 1, 0 <unfinished ...>\n123 <... sendmmsg resumed>) = -1 ENETUNREACH')

    def test_interleaved_probe_results_are_reassembled(self):
        unfinished = PROBE.split(" = ")[0] + " <unfinished ...>\n"
        trace = unfinished + LOCAL.replace("123 ", "124 ") + "123 <... connect resumed>) = -1 ENETUNREACH\n"
        self.assertEqual(audit(trace), ({"127.0.0.1"}, 1))
        with self.assertRaisesRegex(RuntimeError, "incomplete"):
            audit(unfinished)

    def test_source_packet_info_is_not_an_outgoing_destination(self):
        trace = LOCAL + '123 sendmsg(8, {msg_name={sin_addr=inet_addr("127.0.0.1")}, msg_control=[{ipi_spec_dst=inet_addr("0.0.0.0")}], msg_iov=[{iov_base=""}]}, 0) = 8'
        self.assertEqual(audit(trace), ({"127.0.0.1"}, 0))
        with self.assertRaisesRegex(RuntimeError, "unexpected Internet destination"):
            audit(LOCAL.replace("127.0.0.1", "0.0.0.0"))
