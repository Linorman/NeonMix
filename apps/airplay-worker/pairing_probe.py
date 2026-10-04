#!/usr/bin/env python3
"""macOS pairing persistence and asymmetric trust probe; no Apple-client claim.

Run through tools/dev. Private keys and synthetic client identities stay in a
temporary project directory; the report contains only checks and binary hashes.
"""
import base64
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import plistlib
import queue
import socket
import subprocess
import tempfile
import threading
import time

from auth_probe import Crypto, pair_verify, pin_setup

ROOT = Path(__file__).resolve().parents[2]
WORKER = ROOT / '.local/airplay/build/neonmix-airplay-worker'
REPORT = ROOT / 'docs/evidence/airplay/pairing-persistence-macos.json'


@contextmanager
def receiver(directory, keyfile, known=(), blocked=()):
    with socket.socket() as media:
        media.bind(('127.0.0.1', 0))
        media.listen(1)
        media.settimeout(5)
        env = dict(os.environ, GST_PLUGIN_SYSTEM_PATH_1_0=str(ROOT / '.local/airplay/plugins'),
                   GST_REGISTRY=str(directory / 'registry.bin'))
        process = subprocess.Popen([str(WORKER)], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, env=env)
        events = queue.Queue()
        attempts = [0]
        def read_events():
            for line in process.stdout:
                value=json.loads(line)
                if value.get('type')=='pairing_request':
                    allowed=attempts[0]<5
                    if allowed:attempts[0]+=1
                    reply=dict(type='pairing_admit',worker_generation=value['worker_generation'],trust_generation=value['trust_generation'],connection_id=value['connection_id'],pairing_request_id=value['pairing_request_id'],allowed=allowed,attempts=attempts[0])
                    process.stdin.write(json.dumps(reply)+'\n');process.stdin.flush()
                elif value.get('type') not in ['pairing_attempt','pairing_ended']:events.put(value)
        reader = threading.Thread(target=read_events, daemon=True)
        reader.start()
        def event(expected):
            value = events.get(timeout=5)
            assert value['type'] == expected, value['type']
            return value
        config = dict(control_version=2,worker_generation=7,trust_generation=1,pairing_remaining_ms=600000,pairing_attempts=0,media_address=f'127.0.0.1:{media.getsockname()[1]}', ipc_token='a'*64,
                      device_id='001122334455', receiver_uuid='00112233-4455-6677-8899-aabbccddeeff',
                      keyfile=str(keyfile), name='NeonMix pairing probe', pin='1234', rtsp_port=0,
                      session_id=1, stream_id=2, stream_epoch=1, format_epoch=1, mapping_id=1,
                      pairing_allowed=True, known_client_keys=list(known), blocked_client_keys=list(blocked))
        connection = None
        try:
            process.stdin.write(json.dumps(config)+'\n')
            process.stdin.flush()
            connection, _ = media.accept()
            connection.settimeout(5)
            token = bytearray()
            while len(token) < 65:
                chunk = connection.recv(65-len(token))
                assert chunk, 'media authentication closed'
                token.extend(chunk)
            assert token == b'a'*64+b'\n'
            ready = event('ready')
            yield ready, event
        finally:
            if process.poll() is None:
                process.stdin.write('{"type":"stop"}\n')
                process.stdin.flush()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            reader.join(2)
            if connection:
                connection.close()
            process.stdin.close()
            process.stdout.close()
            process.stderr.close()
            assert process.returncode == 0, 'worker exited unsuccessfully'


@contextmanager
def control(port):
    with socket.create_connection(('127.0.0.1', port), timeout=4) as stream:
        sequence = 0
        def request(method, path, body=b''):
            nonlocal sequence
            sequence += 1
            content = 'Content-Type: application/x-apple-binary-plist\r\n' if body else ''
            stream.sendall(f'{method} {path} RTSP/1.0\r\nCSeq: {sequence}\r\nContent-Length: {len(body)}\r\n{content}\r\n'.encode()+body)
            data = b''
            while b'\r\n\r\n' not in data:
                chunk = stream.recv(4096)
                assert chunk, 'RTSP response closed'
                data += chunk
                assert len(data) <= 16384, 'RTSP header limit'
            header, payload = data.split(b'\r\n\r\n', 1)
            length = next((int(line.split(b':', 1)[1]) for line in header.split(b'\r\n')
                           if line.lower().startswith(b'content-length:')), 0)
            assert 0 <= length <= 16384
            while len(payload) < length:
                chunk = stream.recv(length-len(payload))
                assert chunk, 'RTSP payload closed'
                payload += chunk
            return int(header.split(b' ')[1]), payload[:length]
        yield request


def main():
    assert os.uname().sysname == 'Darwin', 'macOS-only probe'
    assert Path(os.environ['TMPDIR']).resolve().is_relative_to(ROOT), 'run through tools/dev'
    crypto = Crypto()
    checks = {}
    with tempfile.TemporaryDirectory(prefix='airplay-pairing-', dir=ROOT / '.local/tmp') as temporary:
        directory = Path(temporary)
        key = directory / 'receiver.pem'
        replacement = directory / 'replacement.pem'
        for path in [key, replacement]:
            subprocess.run(['/opt/homebrew/opt/openssl@3/bin/openssl', 'genpkey', '-algorithm',
                            'ED25519', '-out', str(path)], check=True, capture_output=True)
            path.chmod(0o600)
        private = os.urandom(32)
        with receiver(directory, key) as (ready, event), control(ready['port']) as request:
            server_public = bytes.fromhex(ready['public_key'])
            public = pin_setup(request, crypto, '1234', private, expected_server_public=server_public)
            registration = event('registered')
            assert registration['client_public_key'] == base64.b64encode(public).decode()
            trust_file = directory / 'trust.json'
            trust_file.write_text(json.dumps({'known_client_keys': [registration['client_public_key']]}))
            pair_verify(request, crypto, private, public, expected_server_public=server_public)
            assert request('POST', '/feedback')[0] == 200
        checks['pin_response_and_signed_verify_bind_same_receiver_key'] = True
        known = json.loads(trust_file.read_text())['known_client_keys']
        with receiver(directory, key, known) as (ready, event), control(ready['port']) as request:
            assert bytes.fromhex(ready['public_key']) == server_public
            pair_verify(request, crypto, private, public, expected_server_public=server_public)
            assert request('POST', '/feedback')[0] == 200
        checks['persisted_client_key_reconnects_after_process_restart_without_pin'] = True
        with receiver(directory, key) as (ready, event):
            with control(ready['port']) as request:
                xpub = crypto.primitive('probe_x_public', os.urandom(32))
                assert request('POST', '/pair-verify', b'\1\0\0\0'+xpub+public)[0] == 403
            time.sleep(.1)
            with control(ready['port']) as request:
                assert request('POST', '/pair-pin-start')[0] == 200
                event('pairing_pin')
                pin_setup(request, crypto, '1234', private, expected_server_public=server_public)
                event('registered')
                pair_verify(request, crypto, private, public, expected_server_public=server_public)
                assert request('POST', '/feedback')[0] == 200
        checks['missing_receiver_record_requires_pin_and_can_be_repaired'] = True
        with receiver(directory, key, known, known) as (ready, event), control(ready['port']) as request:
            xpub = crypto.primitive('probe_x_public', os.urandom(32))
            assert request('POST', '/pair-verify', b'\1\0\0\0'+xpub+public)[0] == 403
        checks['revocation_overrides_persisted_known_key'] = True
        with receiver(directory, replacement, known) as (ready, event), control(ready['port']) as request:
            assert bytes.fromhex(ready['public_key']) != server_public
            try:
                pair_verify(request, crypto, private, public, expected_server_public=server_public)
            except AssertionError as error:
                assert str(error) == 'receiver pair-verify identity changed'
            else:
                raise AssertionError('cached receiver key accepted a replacement identity')
        checks['copying_client_list_cannot_repair_changed_receiver_identity'] = True
    report = dict(platform='macOS', passed=True, checks=checks,
                  worker_sha256=hashlib.sha256(WORKER.read_bytes()).hexdigest(),
                  fixture_files_removed=not directory.exists(),
                  scope='synthetic SRP and mutually authenticated pair-verify across worker restarts',
                  limitations=['does not control the iPhone PIN UI', 'does not replace real Apple-source retesting'])
    REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps(report, ensure_ascii=False))


if __name__ == '__main__':
    main()
