"""Disposable HTTP boundary experiment for #2; not the production #10 gateway."""
import asyncio
from http.cookies import SimpleCookie
import secrets
import time
from urllib.parse import urlsplit

from aiohttp import ClientSession, ClientTimeout, DummyCookieJar, WSMsgType, web
from multidict import CIMultiDict

COOKIE = "_wipsnet_session"
HOP = {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
       "te", "trailer", "transfer-encoding", "upgrade"}
START = """<!doctype html><title>Wipsnet browser experiment</title>
<p id="status">Opening local service…</p>
<script>
const token = location.hash.slice(1);
history.replaceState(null, '', location.pathname);
fetch('/_wipsnet/session', {method:'POST',headers:{Authorization:'Bearer '+token}})
.then(r => {if(!r.ok) throw Error(); location.replace('/');})
.catch(() => document.querySelector('#status').textContent = 'Access expired. Open a fresh link.');
</script>"""


def clean_headers(headers):
    excluded = HOP | {s.strip().lower() for s in headers.get("Connection", "").split(",")}
    return CIMultiDict((k, v) for k, v in headers.items()
                      if k.lower() not in excluded and not k.lower().startswith("x-forwarded-")
                      and k.lower() != "forwarded")


class Gateway:
    """One fixed loopback upstream and one exact public authority per instance."""
    def __init__(self, upstream, hostname):
        parsed = urlsplit(upstream)
        if (parsed.scheme != "http" or parsed.hostname != "127.0.0.1"
                or not parsed.port or parsed.path or parsed.query or parsed.fragment
                or parsed.username or parsed.password):
            raise ValueError("spike upstream must be http://127.0.0.1:PORT")
        if not hostname.endswith(".localhost") or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-." for c in hostname):
            raise ValueError("use a development .localhost hostname")
        self.upstream = upstream
        self.hostname = hostname
        self.origin = None
        self.session = None
        self.token = secrets.token_urlsafe(32)
        self.token_expires = time.monotonic() + 60
        self.credential = secrets.token_urlsafe(32)
        self.expires = 0

    def app(self):
        app = web.Application(client_max_size=65536, handler_args={"handler_cancellation": True})
        app.router.add_route("*", "/{path:.*}", self.handle)
        app.cleanup_ctx.append(self.client_context)
        return app

    async def client_context(self, app):
        async with ClientSession(auto_decompress=False, cookie_jar=DummyCookieJar(),
                                 timeout=ClientTimeout(total=30)) as session:
            self.session = session
            yield

    async def start(self):
        self.runner = web.AppRunner(self.app(), access_log=None, shutdown_timeout=1)
        await self.runner.setup()
        self.site = web.TCPSite(self.runner, "127.0.0.1", 0)
        await self.site.start()
        self.port = self.site._server.sockets[0].getsockname()[1]
        self.origin = f"http://{self.hostname}:{self.port}"
        return self

    async def stop(self):
        await self.runner.cleanup()

    @property
    def open_url(self):
        return f"{self.origin}/_wipsnet/start#{self.token}"

    async def handle(self, request):
        # Check authority even for landing/bootstrap paths. No DNS trust or wildcard Host.
        if (request.headers.getall("Host", []) != [self.origin[7:]]
                or request.rel_url.is_absolute() or request.raw_path.startswith("//")):
            return web.Response(status=421, text="Unknown local authority")
        origin = request.headers.getall("Origin", [])
        site = request.headers.get("Sec-Fetch-Site")
        if request.path == "/_wipsnet/start" and request.method == "GET":
            top_navigation = (request.headers.get("Sec-Fetch-Mode") == "navigate"
                              and request.headers.get("Sec-Fetch-Dest") == "document")
            if site not in (None, "none", "same-origin") and not top_navigation:
                return web.Response(status=403, text="Bootstrap embedding denied")
            return web.Response(text=START, content_type="text/html",
                                headers={"Cache-Control": "no-store", "Referrer-Policy": "no-referrer",
                                         "Content-Security-Policy": "default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'"})
        if origin and origin != [self.origin]:
            return web.Response(status=403, text="Origin denied")
        if site not in (None, "none", "same-origin"):
            return web.Response(status=403, text="Cross-site request denied")
        if request.path == "/_wipsnet/session":
            if (request.method != "POST" or origin != [self.origin] or not self.token
                    or time.monotonic() >= self.token_expires
                    or not secrets.compare_digest(request.headers.get("Authorization", ""), "Bearer " + self.token)):
                return web.Response(status=403, text="Bootstrap denied")
            self.token = None
            self.expires = time.monotonic() + 600
            response = web.Response(status=204, headers={"Cache-Control": "no-store"})
            # HTTP experiment only. Secure/__Host- requirements remain a release gate.
            response.set_cookie(COOKIE, self.credential, httponly=True, samesite="Strict", max_age=600, path="/")
            return response
        if request.path.startswith("/_wipsnet/"):
            return web.Response(status=404)
        raw_cookies = request.headers.getall("Cookie", [])
        occurrences = [part.strip() for header in raw_cookies for part in header.split(";")
                       if part.strip().partition("=")[0] == COOKIE]
        if (len(occurrences) != 1 or not secrets.compare_digest(
                occurrences[0], f"{COOKIE}={self.credential}") or time.monotonic() >= self.expires):
            return web.Response(status=401, text="Open the service from Wipsnet")
        upgrade = request.headers.get("Upgrade", "").lower() == "websocket"
        if (request.method not in ("GET", "HEAD") or upgrade) and origin != [self.origin]:
            return web.Response(status=403, text="Same-origin caller required")
        headers = clean_headers(request.headers)
        headers["Cookie"] = "; ".join(part.strip() for header in raw_cookies
                                    for part in header.split(";")
                                    if part.strip().partition("=")[0] != COOKIE)
        headers.popall("Sec-Fetch-Site", None)
        # Keep the browser-facing Host/Origin pair; upstream selection stays fixed.
        url = self.upstream + request.raw_path
        try:
            async with asyncio.timeout(min(30, self.expires - time.monotonic())):
                if upgrade:
                    return await self.websocket(request, url, headers)
                body = await request.read()
                async with self.session.request(request.method, url, headers=headers,
                                                data=body, allow_redirects=False) as upstream:
                    outgoing = clean_headers(upstream.headers)
                    outgoing.popall("Content-Length", None)
                    cookies = outgoing.popall("Set-Cookie", [])
                    location = outgoing.get("Location", "")
                    if location.startswith(self.upstream + "/"):
                        outgoing["Location"] = self.origin + location[len(self.upstream):]
                    response = web.StreamResponse(status=upstream.status, headers=outgoing)
                    for value in cookies:
                        parsed = SimpleCookie()
                        parsed.load(value)
                        for key, cookie in parsed.items():
                            if key == COOKIE:
                                continue
                            cookie["domain"] = ""  # constrain app cookies to this service host
                            response.headers.add("Set-Cookie", cookie.OutputString())
                    response.headers["Referrer-Policy"] = "no-referrer"
                    await response.prepare(request)
                    async for chunk in upstream.content.iter_chunked(16384):
                        await response.write(chunk)
                    await response.write_eof()
                    return response
        except (OSError, asyncio.TimeoutError):
            return web.Response(status=502, text="Backend unavailable or experiment timeout")

    async def websocket(self, request, url, headers):
        protocols = tuple(p.strip() for p in request.headers.get("Sec-WebSocket-Protocol", "").split(",") if p.strip())
        for key in list(headers):
            if key.lower().startswith("sec-websocket-"):
                del headers[key]
        async with self.session.ws_connect(url, headers=headers, protocols=protocols,
                                           max_msg_size=65536) as backend:
            browser = web.WebSocketResponse(protocols=(backend.protocol,) if backend.protocol else (),
                                            max_msg_size=65536)
            await browser.prepare(request)
            async def copy(source, destination):
                async for message in source:
                    if message.type == WSMsgType.TEXT:
                        await destination.send_str(message.data)
                    elif message.type == WSMsgType.BINARY:
                        await destination.send_bytes(message.data)
                    else:
                        break
            tasks = [asyncio.create_task(copy(browser, backend)),
                     asyncio.create_task(copy(backend, browser))]
            try:
                await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            finally:
                for task in tasks:
                    task.cancel()
                await asyncio.gather(*tasks, return_exceptions=True)
                await browser.close()
            return browser
