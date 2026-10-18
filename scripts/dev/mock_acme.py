# Mock de "Acme", la API ficticia de los ejemplos, para probar el Runner y el panel Agente a mano (no es
# parte del producto). Sirve el gateway de Acme en 127.0.0.1:3080 y su API en 127.0.0.1:3081 (las URLs
# del entorno falso de tests/fixtures), con las mismas respuestas que el test de aceptación del runner
# (crates/runpost-core/src/runner/tests.rs):
#   - token "fake-view-token": solo lectura -> la sonda sign da 401 ApiError.
#   - cualquier otro token (p. ej. "fake-spend-token" o el del entorno falso): puede firmar ->
#     sign llega al error local del gateway "key must be an object" (500).
#   - sin token o con el inválido de la colección: 401 en sign; keys/local y encrypt dan 200.
#   - DELETE /v1/<currency>/accounts/<id> en la API (la petición protegida de la demo del panel
#     Agente): 200 con token, 401 sin él.
# Sirve igual para Postman: importa la colección y el entorno falso, elige tokens-matriz-fake.csv en
# su Collection Runner y compara la tabla con la de RunPost.
# Uso: python scripts/dev/mock_acme.py
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

VIEW = 'fake-view-token'


def token_of(headers):
    auth = headers.get('Authorization') or ''
    return auth[len('Bearer '):].strip() if auth.startswith('Bearer ') else ''


class Base(BaseHTTPRequestHandler):
    def send(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def read_body(self):
        length = int(self.headers.get('Content-Length') or 0)
        if length:
            self.rfile.read(length)

    def log_message(self, *args):
        pass


class Gateway(Base):
    def do_GET(self):
        if self.path.startswith('/v1/ping'):
            self.send(200, {'status': 'gateway is ok!'})
        else:
            self.send(404, {'error': 'not found'})

    def do_POST(self):
        self.read_body()
        token = token_of(self.headers)
        if self.path.endswith('/keys/local'):
            self.send(200, {'pub': 'pub-fake', 'prv': 'prv-fake'})
        elif self.path == '/v1/encrypt':
            self.send(200, {'encrypted': '{"iv":"fake"}'})
        elif self.path.endswith('/sign'):
            if not token or token.startswith('TOKEN_INVALIDO'):
                self.send(401, {'error': 'unauthorized'})
            elif token == VIEW:
                self.send(401, {'error': 'unauthorized', 'name': 'ApiError'})
            else:
                self.send(500, {'error': 'key must be an object', 'name': 'Error'})
        else:
            self.send(404, {'error': 'not found'})


class Api(Base):
    def do_GET(self):
        token = token_of(self.headers)
        if not self.path.startswith('/v1/session'):
            self.send(404, {'error': 'not found'})
        elif not token:
            self.send(401, {'error': 'unauthorized'})
        else:
            scope = ['accounts_read'] if token == VIEW else ['accounts_read', 'accounts_sign']
            self.send(200, {'session': {'scope': scope, 'expires': '2026-12-31T00:00:00.000Z', 'ipRestrict': []}})

    def do_DELETE(self):
        parts = self.path.split('?')[0].strip('/').split('/')
        if len(parts) != 4 or parts[0] != 'v1' or parts[2] != 'accounts':
            self.send(404, {'error': 'not found'})
        elif not token_of(self.headers):
            self.send(401, {'error': 'unauthorized'})
        else:
            self.send(200, {'id': parts[3], 'currency': parts[1], 'deleted': True})


servers = [ThreadingHTTPServer(('127.0.0.1', 3080), Gateway), ThreadingHTTPServer(('127.0.0.1', 3081), Api)]
for server in servers[1:]:
    threading.Thread(target=server.serve_forever, daemon=True).start()
print('mock Acme: gateway http://127.0.0.1:3080 · API http://127.0.0.1:3081 (Ctrl+C para salir)', flush=True)
servers[0].serve_forever()
