# Mock mínimo del gateway de Acme (la API ficticia de los ejemplos) para capturas de la UI (no es parte
# del producto): el ping responde y la sonda sign da el 500 de "firma local sin clave".
# Uso: python scripts/dev/mock_gateway.py [puerto=3080]
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

class Handler(BaseHTTPRequestHandler):
    def _send(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path.startswith('/v1/ping'):
            self._send(200, {'status': 'gateway is ok!'})
        else:
            self._send(404, {'error': 'not found'})

    def do_POST(self):
        length = int(self.headers.get('Content-Length', 0))
        self.rfile.read(length)
        if self.path.endswith('/sign'):
            self._send(500, {'error': 'key must be an object', 'name': 'Error', 'requestId': 'req_0000000000000001',
                             'context': {'currency': 'usd', 'accountId': 'acc_0123456789abcdef'}})
        else:
            self._send(401, {'error': 'unauthorized'})

    def log_message(self, *args):
        pass

HTTPServer(('127.0.0.1', int(sys.argv[1]) if len(sys.argv) > 1 else 3080), Handler).serve_forever()
