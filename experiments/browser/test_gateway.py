import asyncio
from http.cookies import SimpleCookie
import time
import unittest

from aiohttp import ClientSession, CookieJar, WSMsgType
from multidict import CIMultiDict
from yarl import URL

from fixtures import Fixture, serve
from gateway import COOKIE, Gateway


class BrowserBoundaryTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.fixture = Fixture()
        self.backend, port = await serve(self.fixture.app())
        self.gateway = await Gateway(f"http://127.0.0.1:{port}", "website.localhost").start()
        self.client = ClientSession()
        self.url = f"http://127.0.0.1:{self.gateway.port}"
        self.headers = {"Host": self.gateway.origin[7:], "Origin": self.gateway.origin}

    async def asyncTearDown(self):
        await self.client.close()
        await self.gateway.stop()
        await self.backend.cleanup()

    async def authorize(self):
        async with self.client.post(self.url + "/_wipsnet/session", headers={
            **self.headers, "Authorization": "Bearer " + self.gateway.token,
        }) as response:
            self.assertEqual(response.status, 204)
            self.assertIn("HttpOnly", response.headers["Set-Cookie"])
            self.assertIn("SameSite=Strict", response.headers["Set-Cookie"])
        self.headers["Cookie"] = f"{COOKIE}={self.gateway.credential}"

    async def test_authorization_is_required_before_backend_and_bootstrap_is_single_use(self):
        async with self.client.get(self.url, headers=self.headers) as response:
            self.assertEqual(response.status, 401)
        token = self.gateway.token
        await self.authorize()
        async with self.client.post(self.url + "/_wipsnet/session", headers={
            **self.headers, "Authorization": "Bearer " + token,
        }) as response:
            self.assertEqual(response.status, 403)
        self.assertEqual(self.fixture.calls, 0)
        self.gateway.expires = time.monotonic() - 1
        async with self.client.get(self.url, headers=self.headers) as response:
            self.assertEqual(response.status, 401)
        self.assertEqual(self.fixture.calls, 0)

    async def test_public_bootstrap_allows_external_top_navigation_but_not_embedding(self):
        async with self.client.get(self.url + "/_wipsnet/start", headers={
            **self.headers, "Sec-Fetch-Site": "cross-site",
            "Sec-Fetch-Mode": "navigate", "Sec-Fetch-Dest": "document",
        }) as response:
            self.assertEqual(response.status, 200)
        async with self.client.get(self.url + "/_wipsnet/start", headers={
            **self.headers, "Sec-Fetch-Site": "cross-site",
            "Sec-Fetch-Mode": "navigate", "Sec-Fetch-Dest": "iframe",
        }) as response:
            self.assertEqual(response.status, 403)
        # The landing page itself never opens the backend or issues authority.
        self.assertEqual(self.fixture.calls, 0)
        async with self.client.post(self.url + "/_wipsnet/session", headers=self.headers) as response:
            self.assertEqual(response.status, 403)
        self.assertEqual(self.fixture.calls, 0)

    async def test_wrong_host_duplicate_host_and_rebinding_cannot_reach_backend(self):
        await self.authorize()
        for host in ("attacker.example", "127.0.0.1", "website.localhost:9"):
            async with self.client.get(self.url, headers={**self.headers, "Host": host}) as response:
                self.assertEqual(response.status, 421)
        duplicate = CIMultiDict(self.headers)
        duplicate.add("Host", "attacker.example")
        async with self.client.get(self.url, headers=duplicate) as response:
            self.assertEqual(response.status, 421)
        self.assertEqual(self.fixture.calls, 0)

    async def test_cross_site_get_post_sibling_origin_null_and_websocket_denied_before_backend(self):
        await self.authorize()
        for origin in ("https://attacker.example", "null", "http://sibling.localhost:5555"):
            for method in ("GET", "POST"):
                async with self.client.request(method, self.url, headers={**self.headers, "Origin": origin}) as response:
                    self.assertEqual(response.status, 403)
            async with self.client.get(self.url + "/ws", headers={
                **self.headers, "Origin": origin, "Upgrade": "websocket",
            }) as response:
                self.assertEqual(response.status, 403)
        for site in ("cross-site", "same-site"):
            async with self.client.get(self.url, headers={**self.headers, "Sec-Fetch-Site": site}) as response:
                self.assertEqual(response.status, 403)
        no_origin = {k: v for k, v in self.headers.items() if k != "Origin"}
        async with self.client.post(self.url + "/login", headers=no_origin) as response:
            self.assertEqual(response.status, 403)
        async with self.client.get(self.url + "/ws", headers={**no_origin, "Upgrade": "websocket"}) as response:
            self.assertEqual(response.status, 403)
        self.assertEqual(self.fixture.calls, 0)

    async def test_normal_navigation_without_origin_and_relative_assets_login_redirect(self):
        await self.authorize()
        no_origin = {k: v for k, v in self.headers.items() if k != "Origin"}
        async with self.client.get(self.url, headers=no_origin) as response:
            self.assertEqual(response.status, 200)
            self.assertIn('/style.css', await response.text())
        async with self.client.get(self.url + "/style.css", headers=no_origin) as response:
            self.assertEqual(response.status, 200)
        async with self.client.post(self.url + "/login", headers=self.headers) as response:
            self.assertEqual(response.status, 200)
            self.assertIn("session=website", response.headers["Set-Cookie"])
        async with self.client.get(self.url + "/absolute", headers=no_origin, allow_redirects=False) as response:
            self.assertEqual(response.headers["Location"], self.gateway.origin + "/")
        async with self.client.get(self.url + "/inspect", headers={
            **self.headers, "Cookie": self.headers["Cookie"] + "; session=website",
            "X-Forwarded-Host": "attacker.example",
        }) as response:
            info = await response.json()
            self.assertEqual(info["cookie"], "session=website")
            self.assertEqual(info["host"], self.gateway.origin[7:])

    async def test_sse_arrives_before_response_finishes_and_disconnect_closes_backend(self):
        await self.authorize()
        async with self.client.get(self.url + "/events", headers=self.headers) as response:
            first = await asyncio.wait_for(response.content.readline(), 0.15)
            self.assertEqual(first, b"data: first\n")
            self.assertIn(b"second", await response.read())
        async with self.client.get(self.url + "/long-events", headers=self.headers) as response:
            self.assertEqual(await response.content.readline(), b"data: live\n")
        await asyncio.wait_for(self.fixture.cancelled.wait(), 1)

    async def test_websocket_text_binary_and_expiry(self):
        await self.authorize()
        async with self.client.ws_connect(self.url + "/ws", headers=self.headers) as ws:
            await ws.send_str("hello")
            self.assertEqual((await asyncio.wait_for(ws.receive(), 1)).data, "hello")
            await ws.send_bytes(b"bytes")
            self.assertEqual((await asyncio.wait_for(ws.receive(), 1)).data, b"bytes")
        self.gateway.expires = time.monotonic() + 0.1
        async with self.client.ws_connect(self.url + "/ws", headers=self.headers) as ws:
            message = await asyncio.wait_for(ws.receive(), 1)
            self.assertIn(message.type, (WSMsgType.CLOSE, WSMsgType.CLOSED))

    async def test_ambiguous_cookie_and_client_selected_upstream_rejected(self):
        await self.authorize()
        async with self.client.get(self.url, headers={
            **self.headers, "Cookie": self.headers["Cookie"] + "; " + self.headers["Cookie"],
        }) as response:
            self.assertEqual(response.status, 401)
        async with self.client.get(self.url + "/_wipsnet/arbitrary-upstream", headers=self.headers) as response:
            self.assertEqual(response.status, 404)
        self.assertEqual(self.fixture.calls, 0)
        with self.assertRaises(ValueError):
            Gateway("http://192.168.1.1:80", "website.localhost")

    async def test_cookie_ports_collide_but_distinct_hostnames_separate_host_only_cookies(self):
        # Exercise aiohttp's actual cookie jar using Set-Cookie, not a policy mock.
        jar = CookieJar(unsafe=True)
        a = URL("http://127.0.0.1:10000")
        b = URL("http://127.0.0.1:10001")
        jar.update_cookies(SimpleCookie("session=website; Path=/"), a)
        self.assertEqual(jar.filter_cookies(b)["session"].value, "website")
        jar.update_cookies(SimpleCookie("session=llama; Path=/"), b)
        self.assertEqual(jar.filter_cookies(a)["session"].value, "llama")
        jar = CookieJar()
        a = URL("http://website.localhost:10000")
        b = URL("http://llama.localhost:10001")
        jar.update_cookies(SimpleCookie("session=website; Path=/"), a)
        self.assertNotIn("session", jar.filter_cookies(b))
        jar.update_cookies(SimpleCookie("session=llama; Path=/"), b)
        self.assertEqual(jar.filter_cookies(a)["session"].value, "website")


if __name__ == "__main__":
    unittest.main()
