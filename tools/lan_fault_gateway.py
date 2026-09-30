"""Project-local test gateway: real TLS/WSS and encrypted UDP are forwarded.

Only transport addresses are remapped for the test; media certificates, keys,
SRTP/SRTCP packets and authenticated control identity remain native end to end.
"""
import http.client
import http.server
import json
import select
import socket
import ssl
import threading
import time


class UdpBridge:
    def __init__(self, sender_port, hub_port):
        self.front = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.back = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.front.bind(('127.0.0.1', 0))
        self.back.bind(('127.0.0.1', 0))
        self.front.connect(('127.0.0.1', sender_port))
        self.back.connect(('127.0.0.1', hub_port))
        self.front.setblocking(False)
        self.back.setblocking(False)
        self.loss_every = 0
        self.blackout = False
        self.delay_ms = 0
        self.media = 0
        self.dropped = 0
        self.forwarded_bytes = 0
        self.feedback_bytes = 0
        self.pending = []
        self.flood = False
        self.injected_datagrams = 0
        self.alive = True
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while self.alive:
            ready, _, _ = select.select([self.front, self.back], [], [], .002)
            for source in ready:
                try:
                    data = source.recv(4097)
                except (BlockingIOError, OSError):
                    continue
                if source is self.back:
                    self.feedback_bytes += len(data)
                    try:
                        self.front.send(data)
                    except OSError:
                        pass
                    continue
                media = len(data) >= 12 and data[0] & 0xc0 == 0x80 and data[1] & 127 == 96
                if media:
                    self.media += 1
                    if self.blackout or (self.loss_every and self.media % self.loss_every == 0):
                        self.dropped += 1
                        continue
                    if self.delay_ms:
                        if len(self.pending) >= 32:
                            self.pending.pop(0)
                            self.dropped += 1
                        self.pending.append((time.monotonic() + self.delay_ms / 1000, data))
                        continue
                try:
                    self.back.send(data)
                    if media:
                        self.forwarded_bytes += len(data)
                except OSError:
                    pass
            if self.flood:
                # The actual connected Hub-facing peer injects malformed SRTP;
                # no certificate, security gate or PCM path is bypassed.
                for _ in range(64):
                    try:
                        self.back.send(b'\x80' * 12)
                        self.injected_datagrams += 1
                    except OSError:
                        break
            now = time.monotonic()
            while self.pending and self.pending[0][0] <= now:
                _, data = self.pending.pop(0)
                try:
                    self.back.send(data)
                    self.forwarded_bytes += len(data)
                except OSError:
                    pass

    def close(self):
        self.alive = False
        self.thread.join(timeout=2)
        self.front.close()
        self.back.close()


class Gateway:
    def __init__(self, hub_port, server_config, directory):
        self.hub_port = hub_port
        self.context = ssl.create_default_context(cadata=server_config['certificate'])
        self.context.minimum_version = ssl.TLSVersion.TLSv1_3
        self.bridges = []
        self.tunnels = []
        self.snapshot_delay = 0
        self.snapshot_requests = 0
        self.drop_snapshots = False
        self.alive = True
        certificate = directory / 'gateway-cert.pem'
        key = directory / 'gateway-key.pem'
        certificate.write_text(server_config['certificate'])
        key.write_text(server_config['private_key'])
        certificate.chmod(0o600)
        key.chmod(0o600)
        gateway = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *args):
                pass  # Never print bearer headers or request contents.

            def do_GET(self):
                if self.headers.get('Upgrade', '').lower() == 'websocket':
                    self.tunnel()
                    return
                if self.path == '/v1/hub':
                    gateway.snapshot_requests += 1
                    time.sleep(gateway.snapshot_delay)
                    if gateway.drop_snapshots:
                        self.close_connection = True
                        return
                self.forward()

            def do_POST(self):
                self.forward()

            def forward(self):
                size = int(self.headers.get('Content-Length', '0'))
                body = self.rfile.read(size) if size else None
                headers = {k: v for k, v in self.headers.items() if k.lower() not in ('host', 'content-length', 'connection')}
                bridge = None
                if body and self.path == '/v1/sessions':
                    data = json.loads(body)
                    if data.get('operation', {}).get('type') == 'start':
                        sender_port = data['operation']['offer']['udp_port']
                        # Reserve the Hub-facing socket before sending the offer.
                        bridge = UdpBridge(sender_port, 9)
                        data['operation']['offer']['udp_port'] = bridge.back.getsockname()[1]
                        body = json.dumps(data).encode()
                conn = http.client.HTTPSConnection('localhost', gateway.hub_port, context=gateway.context, timeout=10)
                try:
                    conn.request(self.command, self.path, body, headers)
                    response = conn.getresponse()
                    result = response.read()
                    if bridge:
                        if response.status == 200:
                            data = json.loads(result)
                            bridge.back.connect(('127.0.0.1', data['media_port']))
                            data['media_port'] = bridge.front.getsockname()[1]
                            result = json.dumps(data).encode()
                            gateway.bridges.append(bridge)
                        else:
                            bridge.close()
                    self.send_response(response.status)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(result)))
                    self.end_headers()
                    self.wfile.write(result)
                except (OSError, ValueError):
                    if bridge:
                        bridge.close()
                    self.close_connection = True
                finally:
                    conn.close()

            def tunnel(self):
                try:
                    back = gateway.context.wrap_socket(socket.create_connection(('127.0.0.1', gateway.hub_port), timeout=5), server_hostname='localhost')
                    request = f'GET {self.path} HTTP/1.1\r\nHost: localhost:{gateway.hub_port}\r\n'
                    for name, value in self.headers.items():
                        if name.lower() != 'host':
                            request += f'{name}: {value}\r\n'
                    back.sendall((request + '\r\n').encode())
                    header = b''
                    while b'\r\n\r\n' not in header:
                        chunk = back.recv(1)
                        if not chunk:
                            raise OSError('Hub closed WebSocket handshake')
                        header += chunk
                    self.connection.sendall(header)
                    if b'101 Switching Protocols' not in header:
                        return
                    gateway.tunnels.append((self.connection, back))
                    while gateway.alive:
                        ready, _, _ = select.select([self.connection, back], [], [], .1)
                        for source in ready:
                            data = source.recv(65536)
                            if not data:
                                return
                            (back if source is self.connection else self.connection).sendall(data)
                except OSError:
                    pass
                finally:
                    self.close_connection = True
                    if 'back' in locals():
                        back.close()

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.minimum_version = ssl.TLSVersion.TLSv1_3
        tls.load_cert_chain(certificate, key)
        self.server.socket = tls.wrap_socket(self.server.socket, server_side=True)
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def disconnect_events(self):
        for front, back in list(self.tunnels):
            for connection in (front, back):
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                    connection.close()
                except OSError:
                    pass
        self.tunnels.clear()

    def close(self):
        self.alive = False
        self.disconnect_events()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        for bridge in self.bridges:
            bridge.close()
