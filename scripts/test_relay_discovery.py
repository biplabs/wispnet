import tomllib
import unittest

from install_relay_discovery import discovery_config


class RelayDiscoveryConfigTests(unittest.TestCase):
    def test_enabling_and_updating_preserves_packet_relay_settings(self):
        initial = 'listen="127.0.0.1:18443"\nhttp_only=true\nmax_connections=64\n'
        first = discovery_config(initial, '/caddy/cert.pem', '/caddy/key.pem')
        second = discovery_config(first, '/renewed/cert.pem', '/renewed/key.pem')
        config = tomllib.loads(second)
        self.assertEqual(config['listen'], '127.0.0.1:18443')
        self.assertEqual(config['max_connections'], 64)
        self.assertEqual(config['quic_listen'], '[::]:7842')
        self.assertEqual(config['quic_certificate'], '/renewed/cert.pem')
        self.assertEqual(config['quic_private_key'], '/renewed/key.pem')
        self.assertEqual(second.count('quic_listen='), 1)

    def test_unrelated_tls_or_public_http_topologies_are_not_reconfigured(self):
        for text in ['listen="0.0.0.0:18443"\nhttp_only=true\nmax_connections=64',
                     'listen="127.0.0.1:8443"\nmax_connections=64']:
            with self.assertRaises(ValueError):
                discovery_config(text, '/cert.pem', '/key.pem')


if __name__ == '__main__':
    unittest.main()
