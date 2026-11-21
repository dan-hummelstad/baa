# Local server for the harness tests: /small and /slow are attachments, /auth wants Basic u:p, anything else is a page.
# Usage: download-server.py [port [cert key]]; with a certificate it speaks TLS (ci.py maps the sites tests load by name to it).
import http.server, socketserver, ssl, sys, time

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
            body = b'<html><head><title>Example Domain</title></head><body>ok</body></html>'
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

tls = None
if len(sys.argv) > 3:
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(sys.argv[2], sys.argv[3])

class Server(http.server.ThreadingHTTPServer):
    # HTTPServer's own server_bind also looks up the host's name, between bind and listen: a slow
    # resolver there leaves the port bound and deaf.
    server_bind = socketserver.TCPServer.server_bind

    def get_request(self):
        sock, addr = super().get_request()
        # The handshake happens at the handler's first read, in its thread: here it would hold up
        # every other connection behind one that Chrome opened early and has not used yet.
        return (tls.wrap_socket(sock, server_side=True, do_handshake_on_connect=False) if tls else sock), addr

    def handle_error(self, request, client_address):
        # A connection the browser dropped, or one that never spoke TLS, is no fault of the server.
        if not isinstance(sys.exc_info()[1], OSError):
            super().handle_error(request, client_address)

Server(('127.0.0.1', int(sys.argv[1]) if len(sys.argv) > 1 else 8123), H).serve_forever()
