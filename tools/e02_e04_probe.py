#!/usr/bin/env python3
"""macOS HTTPS/WSS + two UDP Senders + explicit BlackHole digital readback.

No default route change, no PCM saved. Temporary lab credentials are deleted.
"""
import argparse
import base64
import hashlib
import http.client
import json
from pathlib import Path
import shutil
import signal
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import time
import uuid
from macos_audio_controls import DeviceControls

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--device', required=True, help='BlackHole output/input stable ID')
parser.add_argument('--soak-seconds', type=int, default=0)
parser.add_argument('--ipv6', action='store_true')
args = parser.parse_args()
if sys.platform != 'darwin':
    parser.error('native verification is macOS only')
binary = ROOT / 'target/release/neonmix-hub'
audio = ROOT / 'target/release/neonmix-audio'
out = ROOT / 'artifacts/e02-e04' / time.strftime('%Y%m%d-%H%M%S')
out.mkdir(parents=True)
lab = Path(tempfile.mkdtemp(prefix='hub-probe-', dir=ROOT / '.local/tmp'))
children, handles = [], []
report = {'platform': sys.platform, 'device': args.device, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'passed': False, 'checks': {}}
device_controls=DeviceControls(args.device.removeprefix('coreaudio:'))
original_rate=device_controls.rate()


def start(name, argv):
    stdout, stderr = (out / f'{name}.jsonl').open('w'), (out / f'{name}.stderr').open('w')
    handles.extend([stdout, stderr])
    child = subprocess.Popen(argv, cwd=ROOT, stdout=stdout, stderr=stderr)
    children.append(child)
    return child


def api(path, method='GET', body=None, credential='admin', token=None):
    conn = http.client.HTTPSConnection('localhost', port, context=context, timeout=5)
    try:
        headers = {'Authorization': 'Bearer ' + (token if token is not None else credentials[credential]['token'])}
        if body is not None:
            headers['Content-Type'] = 'application/json'
        conn.request(method, path, json.dumps(body) if body is not None else None, headers)
        response = conn.getresponse()
        data = response.read()
        try:
            value = json.loads(data)
        except json.JSONDecodeError:
            value = {'body': data.decode()[:100]}
        return response.status, value
    finally:
        conn.close()


def snapshot():
    status, state = api('/v1/hub')
    assert status == 200
    return state


def wait_streams(count):
    until = time.monotonic() + 8
    while time.monotonic() < until:
        state = snapshot()
        if len(state['streams']) == count and all(s['status'] in ('playing', 'network_degraded') for s in state['sessions'].values() if s['status'] in ('buffering', 'playing', 'network_degraded')):
            return state
        time.sleep(.1)
    raise AssertionError(f'expected {count} playing streams')


def command(operation, credential='admin', revision=None, expected=200):
    body = {'request_id': str(uuid.uuid4()), 'expected_revision': snapshot()['revision'] if revision is None else revision, 'operation': operation}
    status, value = api('/v1/commands', 'POST', body, credential)
    assert status == expected, (status, value)
    return body, value


def capture(name, frequency=None, silence=False, seconds=2):
    time.sleep(.25)  # settle 5 ms ramps and the bounded native output buffer
    child = start(name, [str(audio), 'capture', '--device', args.device, '--rate', '48000', '--period', '256', '--seconds', str(seconds)])
    assert child.wait(timeout=seconds + 8) == 0
    rows = [json.loads(line) for line in (out / f'{name}.jsonl').read_text().splitlines()]
    final = next(r for r in rows if r['event'] == 'capture_complete')
    stats, measurement = final['stats'], final['measurement']
    assert stats['errors'] == stats['callback_over_budget'] == 0
    if silence:
        assert measurement['peak'] < 1e-7
    else:
        assert measurement['peak'] > .001
        if frequency is not None:
            assert abs(measurement['estimated_frequency_hz'] - frequency) < 1.0
    report['checks'][name] = final
    return measurement


def websocket(after):
    conn = context.wrap_socket(socket.create_connection(('localhost', port), timeout=5), server_hostname='localhost')
    key = base64.b64encode(uuid.uuid4().bytes).decode()
    request = f'GET /v1/events?after={after} HTTP/1.1\r\nHost: localhost:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {credentials["admin"]["token"]}\r\n\r\n'
    conn.sendall(request.encode())
    response = b''
    while b'\r\n\r\n' not in response:
        response += conn.recv(1)
    assert b'101 Switching Protocols' in response
    accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest())
    assert accept in response
    return conn


def ws_json(conn):
    def read(n):
        value = b''
        while len(value) < n:
            part = conn.recv(n - len(value))
            if not part:
                raise EOFError('WebSocket closed')
            value += part
        return value
    header = read(2)
    assert header[0] & 15 == 1 and header[1] & 128 == 0
    size = header[1] & 127
    if size == 126:
        size = struct.unpack('!H', read(2))[0]
    elif size == 127:
        size = struct.unpack('!Q', read(8))[0]
    assert size < 200000
    return json.loads(read(size))


try:
    device_controls.set_rate(48000)
    subprocess.run([str(binary), 'init', '--directory', str(lab), '--output', args.device], check=True, stdout=subprocess.DEVNULL)
    credentials = {name: json.loads((lab / f'{name}.json').read_text()) for name in ('admin', 'sender-a', 'sender-b')}
    context = ssl.create_default_context(cadata=credentials['admin']['certificate'])
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    with socket.socket(socket.AF_INET6 if args.ipv6 else socket.AF_INET) as free:
        free.bind(('::1' if args.ipv6 else '127.0.0.1', 0))
        port = free.getsockname()[1]
    listen = f'[::1]:{port}' if args.ipv6 else f'127.0.0.1:{port}'
    hub = start('hub', [str(binary), 'serve', '--config', str(lab / 'server.json'), '--listen', listen])
    for attempt in range(80):
        try:
            initial = snapshot()
            break
        except (OSError, AssertionError):
            if hub.poll() is not None:
                raise RuntimeError('Hub failed to start')
            time.sleep(.1)
    else:
        raise TimeoutError('Hub startup timeout')
    assert api('/v1/hub', token='unknown')[0] == 401
    report['checks']['unauthenticated'] = 401
    ws = websocket(initial['revision'])
    sender_a = start('sender-a', [str(binary), 'send', '--credential', str(lab / 'sender-a.json'), '--hub', f'https://localhost:{port}', '--seconds', '90', '--frequency', '437'])
    state = wait_streams(1)
    a = next(iter(state['streams'].values()))
    single = capture('single', 437)
    sender_b = start('sender-b', [str(binary), 'send', '--credential', str(lab / 'sender-b.json'), '--hub', f'https://localhost:{port}', '--seconds', '90', '--frequency', '659'])
    state = wait_streams(2)
    b = next(s for s in state['streams'].values() if s['id'] != a['id'])
    capture('dual')
    if args.soak_seconds:
        capture('dual_soak', seconds=args.soak_seconds)
        diagnostics = api('/v1/diagnostics')[1]
        assert diagnostics['underrun_frames'] == 0
        assert diagnostics['output_stats']['errors'] == diagnostics['output_stats']['callback_over_budget'] == 0
        assert all(r['queue_drops'] == 0 and not r['pcm_timing_gaps'] for r in diagnostics['receivers'])
        report['checks']['dual_soak_diagnostics'] = diagnostics
    mix_a = {'type': 'stream_mix', 'stream_id': a['id'], 'gain_db': None, 'muted': None, 'solo': True}
    command(mix_a, 'sender-a', expected=403)
    command({'type': 'output_mix', 'gain_db': -9, 'muted': None}, 'sender-a', expected=403)
    command({'type': 'stream_mix', 'stream_id': b['id'], 'gain_db': -6, 'muted': None, 'solo': None}, 'sender-a', expected=403)
    command({'type': 'output_mix', 'gain_db': -9, 'muted': None}, revision=0, expected=409)
    report['checks']['permissions_and_revision'] = True
    own = {'type': 'stream_mix', 'stream_id': a['id'], 'gain_db': None, 'muted': True, 'solo': None}
    body, receipt = command(own, 'sender-a')
    status, retry = api('/v1/commands', 'POST', body, 'sender-a')
    assert status == 200 and retry['receipt']['revision'] == receipt['receipt']['revision']
    capture('mute_a_only_b', 659)
    command({**own, 'muted': False})
    command(mix_a)
    capture('solo_a', 437)
    command({**mix_a, 'stream_id': b['id']})
    capture('multi_solo')
    command({'type': 'output_mix', 'gain_db': None, 'muted': True})
    before = api('/v1/diagnostics')[1]
    capture('master_mute', silence=True)
    after = api('/v1/diagnostics')[1]
    assert all(new['pcm_frames'] > old['pcm_frames'] for old, new in zip(before['receivers'], after['receivers']))
    command({'type': 'output_mix', 'gain_db': None, 'muted': False})
    command({**mix_a, 'solo': False})
    command({**mix_a, 'stream_id': b['id'], 'solo': False})
    command({'type': 'revoke', 'device_id': a['device_id']})
    assert api('/v1/hub', credential='sender-a')[0] == 401
    capture('revoke_a_only_b', 659)
    command({'type': 'disconnect', 'device_id': b['device_id']})
    capture('admin_disconnect', silence=True)
    offer = state['sessions'][b['session_id']]['offer']
    command({'type': 'start', 'offer': offer}, 'sender-b', expected=403)
    command({'type': 'allow_playback', 'device_id': b['device_id']})
    assert not snapshot()['streams']
    report['checks']['allow_does_not_restart_sender'] = True
    final = snapshot()
    revisions = []
    ws.settimeout(3)
    while not revisions or revisions[-1] < final['revision']:
        revisions.append(ws_json(ws)['revision'])
    assert revisions == sorted(set(revisions)) and revisions[0] == initial['revision'] + 1
    ws.close()
    report['checks']['wss_revisions'] = revisions
    report['diagnostics'] = api('/v1/diagnostics')[1]
    assert not report['diagnostics']['errors']
    saved = json.loads((lab / 'state.json').read_text())
    assert saved['devices'][a['device_id']]['revoked']
    hub.send_signal(signal.SIGINT)
    assert hub.wait(timeout=5) == 0
    restarted = start('hub-restarted', [str(binary), 'serve', '--config', str(lab / 'server.json'), '--listen', listen])
    for attempt in range(60):
        try:
            restored = snapshot()
            break
        except OSError:
            time.sleep(.1)
    assert restored['hub_id'] == initial['hub_id'] and not restored['streams']
    assert api('/v1/hub', credential='sender-a')[0] == 401
    report['checks']['restart_preserves_revocation'] = True
    report['passed'] = True
except Exception as error:
    report['error'] = str(error)
finally:
    for child in children:
        if child.poll() is None:
            child.send_signal(signal.SIGINT)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
    for handle in handles:
        handle.close()
    shutil.rmtree(lab)
    device_controls.set_rate(original_rate)
    (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'evidence': str(out), 'error': report.get('error')}, indent=2))
raise SystemExit(0 if report['passed'] else 1)
