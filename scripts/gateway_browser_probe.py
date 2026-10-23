#!/usr/bin/env python3
"""Browser worker; capability URLs arrive via stdin, never process arguments."""

import json
import sys

# Runs in the service origin on both Chromium and headless Firefox.
PROBE = r"""async function probe(other) {
 const check=(v,name)=>{if(!v)throw new Error(name)};
 const fetchText=async p=>{const r=await fetch(p);return [r.status,await r.text()]};
 check((await fetchText('/cookie'))[0]===200,'cookie response');
 localStorage.setItem('isolation', location.host);
 const inspected=JSON.parse((await fetchText('/inspect'))[1]);
 check(!Object.keys(inspected).some(k=>k.toLowerCase().startsWith('x-forwarded-')),'forwarded');
 check(!JSON.stringify(inspected).includes('_wispnet_session'),'credential leaked');
 check(!document.cookie.includes('_wispnet_session'),'HttpOnly');
 check((await fetch('/login')).status===200,'relative login');
 check((await fetch('/secure')).status===502,'secure cookie rejection');
 check((await fetch('/external')).status===502,'external redirect rejection');
 check((await fetchText('/events'))[1]==='data: hello\n\n','chunked SSE');
 let sw=false;try {await navigator.serviceWorker.register('/sw.js')}catch(e){sw=true}
 check(sw,'service worker blocked');
 let cross=false;try{await fetch(other+'/attack',{credentials:'include'})}catch(e){cross=true}
 check(cross,'cross-origin fetch blocked');
 const ws=await new Promise(resolve=>{const w=new WebSocket(location.origin.replace('http:','ws:')+'/ws');const t=setTimeout(()=>{w.close();resolve(false)},5000);w.onopen=()=>w.send('hello');w.onmessage=e=>{clearTimeout(t);w.close();resolve(e.data==='hello')};w.onerror=()=>{clearTimeout(t);resolve(false)}});
 check(ws,'same-origin WebSocket');
 const denied=await new Promise(resolve=>{const w=new WebSocket(other.replace('http:','ws:')+'/attackws');const t=setTimeout(()=>{w.close();resolve(false)},5000);w.onopen=()=>{clearTimeout(t);w.close();resolve(false)};w.onerror=()=>{clearTimeout(t);resolve(true)}});
 check(denied,'cross-origin WebSocket');
 const frame=document.createElement('iframe');frame.name='attack-sink';frame.src=other+'/attack';document.body.append(frame);
 const form=document.createElement('form');form.method='POST';form.action=other+'/attack';form.target=frame.name;document.body.append(form);form.submit();
 await new Promise(r=>setTimeout(r,500));form.remove();frame.remove();
 return {cookie:document.cookie,storage:localStorage.getItem('isolation'),origin:location.origin};
}"""


def chromium(urls):
    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        browser = p.chromium.launch(
            executable_path="/usr/bin/chromium",
            headless=True,
            args=["--no-proxy-server"],
        )
        context = browser.new_context()
        pages = [context.new_page(), context.new_page()]
        for page, url in zip(pages, urls):
            page.goto(url)
            page.wait_for_url("**/")
        results = [
            page.evaluate("(" + PROBE + ")", pages[1 - i].url.rstrip("/"))
            for i, page in enumerate(pages)
        ]
        assert results[0]["origin"] != results[1]["origin"]
        for i, page in enumerate(pages):
            assert (
                page.evaluate('localStorage.getItem("isolation")')
                == results[i]["storage"]
            )
            assert page.evaluate("document.cookie") == results[i]["cookie"]
        assert results[0]["cookie"] != results[1]["cookie"], (
            "application cookies crossed origins"
        )
        assert not any(c["name"] == "_wispnet_session" for c in context.cookies())
        print(
            "Chromium "
            + browser.version
            + ": two origins, isolated cookies/storage, login, SSE, WebSocket; cross-origin fetch/WS, Secure cookies, external redirect and service worker denied"
        )
        browser.close()


def firefox(urls):
    import os
    from pathlib import Path
    import shutil
    import socket
    import subprocess
    import tempfile
    import time
    import urllib.request

    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    root = (
        Path.home() / "snap/firefox/common" / ("wispnet-security-" + str(os.getpid()))
    )
    root.mkdir(mode=0o700)
    log = tempfile.TemporaryFile()
    proc = subprocess.Popen(
        [
            "/snap/bin/geckodriver",
            "--port",
            str(port),
            "--host",
            "127.0.0.1",
            "--profile-root",
            str(root),
            "--log",
            "error",
        ],
        stdout=log,
        stderr=log,
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def call(method, path, data=None):
        req = urllib.request.Request(
            "http://127.0.0.1:" + str(port) + path,
            data=json.dumps(data).encode() if data is not None else None,
            method=method,
            headers={"Content-Type": "application/json"},
        )
        try:
            with opener.open(req, timeout=45) as r:
                value = json.load(r)["value"]
        except Exception:
            raise RuntimeError(
                "WebDriver request failed; response suppressed to protect bootstrap URLs"
            ) from None
        if isinstance(value, dict) and "error" in value:
            raise RuntimeError("WebDriver " + value["error"])
        return value

    session = None
    try:
        deadline = time.monotonic() + 15
        while True:
            try:
                call("GET", "/status")
                break
            except RuntimeError:
                assert time.monotonic() < deadline
                time.sleep(0.1)
        result = call(
            "POST",
            "/session",
            {
                "capabilities": {
                    "alwaysMatch": {
                        "browserName": "firefox",
                        "moz:firefoxOptions": {
                            "args": ["-headless"],
                            "prefs": {"network.proxy.type": 0},
                        },
                    }
                }
            },
        )
        session = result["sessionId"]
        path = "/session/" + session
        handles = [call("GET", path + "/window")]
        handles.append(call("POST", path + "/window/new", {"type": "tab"})["handle"])
        origins = []
        for handle, url in zip(handles, urls):
            call("POST", path + "/window", {"handle": handle})
            call("POST", path + "/url", {"url": url})
            deadline = time.monotonic() + 25
            while True:
                state = call(
                    "POST",
                    path + "/execute/sync",
                    {
                        "script": "return {path:location.pathname,origin:location.origin}",
                        "args": [],
                    },
                )
                if state["path"] == "/":
                    break
                assert time.monotonic() < deadline
                time.sleep(0.1)
            origins.append(state["origin"])
        results = []
        for i, handle in enumerate(handles):
            call("POST", path + "/window", {"handle": handle})
            value = call(
                "POST",
                path + "/execute/async",
                {
                    "script": PROBE
                    + "\nconst done=arguments[arguments.length-1];probe(arguments[0]).then(done).catch(e=>done({failure:e.message}));",
                    "args": [origins[1 - i]],
                },
            )
            assert "failure" not in value, value
            results.append(value)
        assert results[0]["cookie"] != results[1]["cookie"]
        for i, handle in enumerate(handles):
            call("POST", path + "/window", {"handle": handle})
            value = call(
                "POST",
                path + "/execute/sync",
                {
                    "script": 'return {cookie:document.cookie,storage:localStorage.getItem("isolation")}',
                    "args": [],
                },
            )
            assert (
                value["cookie"] == results[i]["cookie"]
                and value["storage"] == results[i]["storage"]
            )
            assert not any(
                c["name"] == "_wispnet_session"
                for c in call("GET", path + "/cookie")
            )
        print(
            "Headless Firefox "
            + result["capabilities"]["browserVersion"]
            + ": two origins, isolated cookies/storage, login, SSE, WebSocket; cross-origin fetch/WS, Secure cookies, external redirect and service worker denied"
        )
    finally:
        if session:
            try:
                call("DELETE", "/session/" + session)
            except RuntimeError:
                pass
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        log.close()
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    try:
        values = json.load(sys.stdin)
        if values["browser"] == "firefox":
            firefox(values["urls"])
        else:
            chromium(values["urls"])
    except Exception as error:
        # Browser diagnostics can contain the capability URL passed to goto().
        print(
            "Browser probe failed ("
            + type(error).__name__
            + "); capability diagnostics suppressed",
            file=sys.stderr,
        )
        sys.exit(1)
