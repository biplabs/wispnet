from pathlib import Path
import tempfile
import unittest
from wispnet_desktop.catalog_cache import CatalogCache, MAX_BYTES


class CacheChecks(unittest.TestCase):
    def test_private_roundtrip_corruption_and_cache_deletion_leave_pins_intact(self):
        with tempfile.TemporaryDirectory() as directory:
            peer = "a" * 64
            cache = CatalogCache(directory)
            entry = {
                "catalog": {
                    "host": peer,
                    "version": 1,
                    "complete": True,
                    "service_count": 1,
                    "generation": "c" * 64,
                    "revision": 2,
                    "services": [
                        {
                            "id": "web.site",
                            "incarnation": "a" * 64,
                            "display_name": "Website",
                            "kind": "http",
                        }
                    ],
                },
                "fetched": 123.5,
            }
            pins = Path(directory) / "peer-pins"
            pins.mkdir()
            pin = pins / (peer + ".json")
            pin.write_text("pin fixture")
            cache.save(peer, entry)
            self.assertEqual(cache.path(peer).stat().st_mode & 0o777, 0o600)
            self.assertEqual(cache.directory.stat().st_mode & 0o777, 0o700)
            self.assertEqual(cache.load(peer)["catalog"], entry["catalog"])
            self.assertNotEqual(cache.load(peer)["state"], "Available")
            other = "b" * 64
            cache.path(other).write_bytes(cache.path(peer).read_bytes())
            cache.path(other).chmod(0o600)
            with self.assertRaises(ValueError):
                cache.load(other)
            cache.path(peer).write_text("{")
            with self.assertRaises(ValueError):
                cache.load(peer)
            cache.save(peer, entry)
            cache.path(peer).chmod(0o644)
            with self.assertRaises(ValueError):
                cache.load(peer)
            cache.path(peer).chmod(0o600)
            cache.path(peer).write_bytes(b"x" * (MAX_BYTES + 1))
            with self.assertRaises(ValueError):
                cache.load(peer)
            cache.remove(peer)
            self.assertIsNone(cache.load(peer))
            self.assertEqual(pin.read_text(), "pin fixture")
            cache.path(peer).symlink_to(pin)
            with self.assertRaises(OSError):
                cache.load(peer)
            self.assertEqual(pin.read_text(), "pin fixture")
            with self.assertRaises(ValueError):
                cache.load("../peer-pins")

    def test_malformed_catalog_never_persists(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = CatalogCache(directory)
            peer = "a" * 64
            for services in [
                [{"id": "<script>"}],
                [
                    {
                        "id": "website",
                        "incarnation": "a" * 64,
                        "display_name": "Website",
                        "kind": "http",
                    },
                    {
                        "id": "website",
                        "incarnation": "a" * 64,
                        "display_name": "Website",
                        "kind": "http",
                    },
                ],
            ]:
                with self.assertRaises(ValueError):
                    cache.save(
                        peer,
                        {
                            "catalog": {
                                "host": peer,
                                "version": 1,
                                "complete": True,
                                "service_count": 1,
                                "generation": "c" * 64,
                                "revision": 1,
                                "services": services,
                            },
                            "fetched": 1,
                        },
                    )
            self.assertIsNone(cache.load(peer))
