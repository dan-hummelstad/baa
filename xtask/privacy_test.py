"""PLAN.md §5.3: run baa behind mitmproxy and fail if it contacts any host
outside baa-privacy's allowlist.

usage: privacy_test.py APP ALLOWLIST [--seconds 60]

APP is the bundled app (baa.app) or its executable. ALLOWLIST is the output of
`cargo run -p baa-privacy --example proxy_allowlist` (baa_privacy::PROXY_ALLOWLIST,
one exact host per line). Needs `mitmdump` on PATH (pip install mitmproxy).

The app opens a local page on 127.0.0.1, which Chromium never proxies, so every
request mitmproxy sees was made by the engine itself or by baa (e.g. filter-list
refreshes) and has to be allowlisted.
"""
import argparse
import http.server
import os
import pathlib
import socket
import subprocess
import sys
import tempfile
import threading
import time


# Logs the target of every proxied request. --ignore-hosts '.*' tunnels TLS
# untouched, so no CA install is needed; the CONNECT host is all we want.
ADDON = """
import os
LOG = os.environ["BAA_HOST_LOG"]
def _log(flow):
    with open(LOG, "a") as f:
        f.write(flow.request.host + "\\n")
def http_connect(flow): _log(flow)
def requestheaders(flow): _log(flow)
"""

PAGE = b"<!doctype html><title>baa privacy test</title><p>Nothing to fetch here.</p>"


def disallowed(hosts, allowlist_text):
    allowed = {line.strip().lower() for line in allowlist_text.splitlines()}
    return sorted({h for h in hosts if h.lower().rstrip(".") not in allowed})


def executable(app):
    app = pathlib.Path(app).resolve()
    return app / "Contents" / "MacOS" / app.stem if app.suffix == ".app" else app


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_port(port, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            socket.create_connection(("127.0.0.1", port), timeout=1).close()
            return
        except OSError:
            time.sleep(0.2)
    sys.exit(f"mitmdump did not start listening on {port}")


class Page(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.end_headers()
        self.wfile.write(PAGE)

    def log_message(self, *args):
        pass


def stop(proc):
    proc.terminate()
    try:
        proc.wait(10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("app")
    parser.add_argument("allowlist", type=pathlib.Path)
    parser.add_argument("--seconds", default=60, type=int)
    args = parser.parse_args()

    allowlist = args.allowlist.read_text()
    if not allowlist.strip():
        sys.exit(f"{args.allowlist} is empty; did the proxy_allowlist example run?")
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="baa-privacy-"))
    (tmp / "addon.py").write_text(ADDON)
    log = tmp / "hosts.log"
    log.touch()

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Page)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    page_url = f"http://127.0.0.1:{server.server_address[1]}/"

    port = free_port()
    proxy = subprocess.Popen(
        ["mitmdump", "-q", "--listen-host", "127.0.0.1", "--listen-port", str(port),
         "--ignore-hosts", ".*", "-s", str(tmp / "addon.py")],
        env={**os.environ, "BAA_HOST_LOG": str(log)},
    )
    try:
        wait_for_port(port)
        app = subprocess.Popen([str(executable(args.app)), f"--proxy-server=http://127.0.0.1:{port}", page_url])
        time.sleep(args.seconds)
        crashed = app.poll()
        stop(app)
    finally:
        stop(proxy)
        server.shutdown()

    hosts = set(log.read_text().split())
    print(f"hosts contacted via proxy: {sorted(hosts) or 'none'}")
    if crashed is not None:
        sys.exit(f"app exited after <{args.seconds}s with code {crashed}; the test proves nothing")
    if bad := disallowed(hosts, allowlist):
        sys.exit(f"contacted hosts outside {args.allowlist.name}: {bad}")
    print("privacy test passed")


if __name__ == "__main__":
    main()
