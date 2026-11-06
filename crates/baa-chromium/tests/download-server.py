# Local server for downloads.txt and asks.txt: /small and /slow are attachments, /auth wants Basic u:p. Usage: download-server.py [port]
import http.server, sys, time

class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/slow'):
            total = 20 * 1024 * 1024
            self.send_response(200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Disposition', 'attachment; filename="baa-slow.bin"')
            self.send_header('Content-Length', str(total))
            self.end_headers()
            chunk = b'x' * 262144
            try:
                for _ in range(total // len(chunk)):
                    self.wfile.write(chunk)
                    time.sleep(0.2)
            except Exception:
                pass
        elif self.path.startswith('/auth'):
            ok = self.headers.get('Authorization') == 'Basic dTpw'
            body = b'<html><head><title>authed</title></head><body>ok</body></html>' if ok else b'denied'
            self.send_response(200 if ok else 401)
            if not ok:
                self.send_header('WWW-Authenticate', 'Basic realm="baa"')
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path.startswith('/small'):
            body = b'baa-download-test\n' * 1000
            self.send_response(200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Disposition', 'attachment; filename="baa-small.bin"')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            body = b'<html><body>ok</body></html>'
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    def log_message(self, *a):
        pass

http.server.ThreadingHTTPServer(('127.0.0.1', int(sys.argv[1]) if len(sys.argv) > 1 else 8123), H).serve_forever()
