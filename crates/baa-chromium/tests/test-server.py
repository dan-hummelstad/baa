# The harness tests' server, one for a whole run (ci.py starts it): /small and /slow are attachments, /auth wants Basic u:p,
# /otp is a page with a one-time-code field, one in an open shadow root and a plain one (codes.expect),
# /mail-seen lists the /mail-* paths asked for so far (mail-document.expect), anything else is a page.
# Usage: test-server.py <cert> <key>. It serves the same over plain HTTP and over TLS (where ci.py maps the sites tests
# load by name) and prints the two ports once both listen.
import http.server, socketserver, ssl, sys, threading, time

MAIL_SEEN = []

class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/mail-') and self.path != '/mail-seen':
            MAIL_SEEN.append(self.path)
        if self.path == '/mail-seen':
            body = f'<html><head><title>mail-seen:{",".join(MAIL_SEEN)}</title></head></html>'.encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path.startswith('/slow'):
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
        elif self.path == '/otp':
            body = (b'<html><head><title>otp</title></head><body>'
                    b'<input id=c autocomplete="one-time-code" style="position:fixed;left:0;top:0;width:300px;height:60px">'
                    b'<div id=h><template shadowrootmode=open><input id=s autocomplete="one-time-code" '
                    b'style="position:fixed;left:0;top:100px;width:300px;height:60px"></template></div>'
                    b'<input id=n style="position:fixed;left:0;top:200px;width:300px;height:60px"></body></html>')
            self.send_response(200)
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
            body = b'<html><head><title>Example Domain</title></head><body><h1>Example Domain</h1></body></html>'
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

class Server(http.server.ThreadingHTTPServer):
    # HTTPServer's own server_bind also looks up the host's name, between bind and listen: a slow
    # resolver there leaves the port bound and deaf.
    server_bind = socketserver.TCPServer.server_bind
    tls = None

    def get_request(self):
        sock, addr = super().get_request()
        # The handshake happens at the handler's first read, in its thread: here it would hold up
        # every other connection behind one that Chrome opened early and has not used yet.
        return (self.tls.wrap_socket(sock, server_side=True, do_handshake_on_connect=False) if self.tls else sock), addr

    def handle_error(self, request, client_address):
        # A connection the browser dropped, or one that never spoke TLS, is no fault of the server.
        if not isinstance(sys.exc_info()[1], OSError):
            super().handle_error(request, client_address)

plain, secure = Server(('127.0.0.1', 0), H), Server(('127.0.0.1', 0), H)
secure.tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
secure.tls.load_cert_chain(sys.argv[1], sys.argv[2])
threading.Thread(target=secure.serve_forever, daemon=True).start()
print(plain.server_address[1], secure.server_address[1], flush=True)
plain.serve_forever()
