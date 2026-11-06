import http.server, time, os

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

http.server.ThreadingHTTPServer(('127.0.0.1', 8123), H).serve_forever()
