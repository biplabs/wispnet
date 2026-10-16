"""Controlled compatibility website. Contains disposable login state only."""
import asyncio
from aiohttp import WSMsgType, web

PAGE = """<!doctype html><title>Wipsnet compatibility website</title>
<link rel="stylesheet" href="/style.css">
<h1>Compatibility website</h1>
<button id="run">Run compatibility checks</button><pre id="results">Ready</pre>
<a href="/absolute">Absolute redirect</a>
<script>
document.querySelector('#run').onclick=async()=>{
 const results=document.querySelector('#results'); results.textContent='Running';
 try{
  const login=await fetch('/login',{method:'POST'}); if(!login.ok)throw Error('login');
  const info=await (await fetch('/inspect')).json();
  const asset=await fetch('/style.css');
  const reader=(await fetch('/events')).body.getReader();
  const first=await reader.read(); const early=new TextDecoder().decode(first.value).includes('first');
  while(!(await reader.read()).done){}
  const ws=await new Promise((resolve,reject)=>{
   const socket=new WebSocket(location.origin.replace('http','ws')+'/ws');
   socket.onopen=()=>socket.send('hello');
   socket.onmessage=e=>{resolve(e.data);socket.close();};socket.onerror=reject;
  });
  const report={login:login.status,cookie:info.cookie,asset:asset.status,
   earlySSE:early,websocket:ws,secureContext:isSecureContext,userAgent:navigator.userAgent};
  results.textContent=JSON.stringify(report,null,2);
  await fetch('/report',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(report)});
 }catch(e){results.textContent='Failed: '+e;}
};
</script>"""


class Fixture:
    def __init__(self, name="website"):
        self.name = name
        self.calls = 0
        self.reports = []
        self.cancelled = asyncio.Event()

    def app(self):
        app = web.Application()
        app.router.add_route("*", "/{path:.*}", self.handle)
        return app

    async def handle(self, request):
        self.calls += 1
        if request.path == "/report" and request.method == "POST":
            self.reports.append(await request.json())
            return web.Response(status=204)
        if request.path == "/":
            return web.Response(text=PAGE, content_type="text/html")
        if request.path == "/style.css":
            return web.Response(text="body{font:18px sans-serif;max-width:900px;margin:40px auto}pre{white-space:pre-wrap}",
                                content_type="text/css")
        if request.path == "/login":
            response = web.Response(text="logged in")
            response.set_cookie("session", self.name, httponly=True, path="/")
            return response
        if request.path == "/inspect":
            return web.json_response({"cookie": request.headers.get("Cookie", ""), "host": request.host,
                                      "origin": request.headers.get("Origin")})
        if request.path == "/absolute":
            return web.Response(status=302, headers={"Location": f"http://127.0.0.1:{request.transport.get_extra_info('sockname')[1]}/"})
        if request.path == "/events":
            response = web.StreamResponse(headers={"Content-Type": "text/event-stream"})
            await response.prepare(request)
            await response.write(b"data: first\n\n")
            await asyncio.sleep(0.25)
            await response.write(b"data: second\n\n")
            return response
        if request.path == "/long-events":
            response = web.StreamResponse(headers={"Content-Type": "text/event-stream"})
            await response.prepare(request)
            try:
                for _ in range(200):
                    await response.write(b"data: live\n\n")
                    await asyncio.sleep(0.01)
            except ConnectionError:
                self.cancelled.set()
                return response
            except asyncio.CancelledError:
                self.cancelled.set()
                raise
            return response
        if request.path == "/ws":
            ws = web.WebSocketResponse()
            await ws.prepare(request)
            async for message in ws:
                if message.type == WSMsgType.TEXT:
                    await ws.send_str(message.data)
                elif message.type == WSMsgType.BINARY:
                    await ws.send_bytes(message.data)
            return ws
        return web.Response(status=404)


async def serve(app):
    runner = web.AppRunner(app, access_log=None, shutdown_timeout=1)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    return runner, site._server.sockets[0].getsockname()[1]
