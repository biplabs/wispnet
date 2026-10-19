"""Launch disposable fixtures and loopback experiment. Stop with Ctrl-C."""
import argparse
import asyncio
import json
import html
import subprocess
import time
import secrets
import signal

from aiohttp import web
from fixtures import Fixture, serve
from gateway import Gateway


async def main(llama_port, website_port, phone):
    runners, gateways = [], []
    fixtures = []
    links = {}
    reverse_ports = []
    handoff_nonce = secrets.token_urlsafe(32)
    handoff_expiry = time.monotonic() + 600
    run_id = secrets.token_hex(4)
    try:
        for name in ("website", "second"):
            fixture = Fixture(name)
            fixtures.append(fixture)
            runner, port = await serve(fixture.app())
            runners.append(runner)
            gateway = await Gateway(f"http://127.0.0.1:{port}", f"{name}-{run_id}.localhost").start()
            gateways.append(gateway)
            links[name] = gateway.open_url
            links[name + "_direct"] = f"http://127.0.0.1:{port}"
        if llama_port:
            gateway = await Gateway(f"http://127.0.0.1:{llama_port}", f"llama-{run_id}.localhost").start()
            gateways.append(gateway)
            links["llama"] = gateway.open_url
        if website_port:
            gateway = await Gateway(f"http://127.0.0.1:{website_port}", f"gike-{run_id}.localhost").start()
            gateways.append(gateway)
            links["gike"] = gateway.open_url
        origin = gateways[0].origin

        async def attacker(request):
            if request.path == "/handoff/" + handoff_nonce:
                if time.monotonic() >= handoff_expiry:
                    return web.Response(status=410, text="Restart the test for a fresh handoff.")
                metadata = {key: request.headers.get(key) for key in ("Sec-Fetch-Site", "Sec-Fetch-Mode", "Sec-Fetch-Dest")}
                print(json.dumps({"handoff_metadata": metadata}), flush=True)
                top_navigation = (request.headers.get("Sec-Fetch-Mode") == "navigate"
                                  and request.headers.get("Sec-Fetch-Dest") == "document")
                if request.headers.get("Sec-Fetch-Site") not in (None, "none", "same-origin") and not top_navigation:
                    return web.Response(status=403)
                entries = []
                for gateway in gateways:
                    gateway.token = secrets.token_urlsafe(32)
                    gateway.token_expires = time.monotonic() + 60
                    entries.append(f'<p><a target="_blank" href="{html.escape(gateway.open_url)}">Open {html.escape(gateway.hostname.split("-")[0])}</a></p>')
                return web.Response(text='<h1>Wispnet USB browser experiment</h1><p>Open a service within 60 seconds. Reload this page for fresh links. On website and second, tap Run compatibility checks.</p>' + ''.join(entries),
                    content_type="text/html", headers={"Cache-Control":"no-store", "Referrer-Policy":"no-referrer",
                    "Content-Security-Policy":"default-src 'none'; frame-ancestors 'none'"})
            if request.path == "/reports/" + handoff_nonce:
                return web.json_response({fixture.name: fixture.reports for fixture in fixtures})

            if request.path == "/counts":
                return web.json_response({"calls": fixtures[0].calls})
            if request.path == "/":
                page = """<!doctype html><title>Unrelated website attack fixture</title>
<h1>Unrelated website</h1><button id="attack">Run cross-site probes</button>
<pre id="results">Ready</pre><iframe name="target" hidden></iframe>
<script>
const target=TARGET;
document.querySelector('#attack').onclick=async()=>{
 const before=(await(await fetch('/counts')).json()).calls;
 const results=document.querySelector('#results');results.textContent='Running';
 await Promise.all([
  fetch(target+'/login',{method:'POST',credentials:'include'}).catch(()=>{}),
  fetch(target+'/inspect',{mode:'no-cors',credentials:'include'}).catch(()=>{}),
  new Promise(resolve=>{const ws=new WebSocket(target.replace('http','ws')+'/ws');
    ws.onerror=()=>resolve(); ws.onopen=()=>{ws.close();resolve();};})
 ]);
 const form=document.createElement('form');form.action=target+'/login';
 form.method='POST';form.target='target';document.body.append(form);form.submit();
 setTimeout(async()=>{
  const after=(await(await fetch('/counts')).json()).calls;
  results.textContent=JSON.stringify({backendRequestDelta:after-before,expected:0,
    userAgent:navigator.userAgent},null,2);
 },300);
};
</script>"""
                return web.Response(text=page.replace("TARGET", json.dumps(origin)), content_type="text/html")
            return web.Response(status=404)
        attack_app = web.Application()
        attack_app.router.add_route("*", "/{path:.*}", attacker)
        runner, port = await serve(attack_app)
        runners.append(runner)
        links["attacker"] = f"http://127.0.0.1:{port}"
        if phone:
            for local_port in [port] + [gateway.port for gateway in gateways]:
                subprocess.run(["adb", "-d", "reverse", "--no-rebind", f"tcp:{local_port}", f"tcp:{local_port}"], check=True, capture_output=True)
                reverse_ports.append(local_port)
            links["phone_handoff"] = f"http://127.0.0.1:{port}/handoff/{handoff_nonce}"
            links["phone_reports"] = f"http://127.0.0.1:{port}/reports/{handoff_nonce}"
        print(json.dumps(links, indent=2), flush=True)
        print("Disposable bootstrap links expire after 60 seconds; restart for fresh links.", flush=True)
        stop = asyncio.Event()
        for sig in (signal.SIGINT, signal.SIGTERM):
            asyncio.get_running_loop().add_signal_handler(sig, stop.set)
        await stop.wait()
    finally:
        for port in reverse_ports:
            subprocess.run(["adb", "-d", "reverse", "--remove", f"tcp:{port}"], capture_output=True)
        for gateway in gateways:
            await gateway.stop()
        for runner in runners:
            await runner.cleanup()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--llama-port", type=int, help="Optional existing loopback llama.cpp port; no process reconfiguration")
    parser.add_argument("--website-port", type=int, help="Optional existing loopback website port")
    parser.add_argument("--phone", action="store_true", help="Temporary adb reverse for one authorized USB Android device; removed on exit")
    args = parser.parse_args()
    asyncio.run(main(args.llama_port, args.website_port, args.phone))
