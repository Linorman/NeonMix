#!/usr/bin/env python3
"""HTTPS/WSS + two UDP Senders + explicit virtual-device digital readback.

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
import shlex
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import time
import traceback
import uuid

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--device', required=True, help='BlackHole or PipeWire test sink stable ID')
parser.add_argument('--soak-seconds', type=int, default=0)
parser.add_argument('--soak-only', action='store_true', help='Run continuous dual-source acceptance without the control scenarios')
parser.add_argument('--ipv6', action='store_true')
parser.add_argument('--linux-peer', help='SSH user@host, using an existing project-local control socket')
parser.add_argument('--peer-workspace', help='Project-local Linux snapshot directory')
parser.add_argument('--peer-audio-rlimits', action='store_true', help='Grant only child-process audio rlimits for a root SSH headless test session')
parser.add_argument('--hub-platform', choices=('macos', 'linux'), default='macos')
parser.add_argument('--macos-host', default='192.168.100.111')
args = parser.parse_args()
if args.soak_only and args.soak_seconds <= 0:
    parser.error('--soak-only requires a positive --soak-seconds')
if args.peer_audio_rlimits and not args.linux_peer:
    parser.error('--peer-audio-rlimits requires --linux-peer')
if args.linux_peer and (sys.platform != 'darwin' or not args.peer_workspace or args.ipv6):
    parser.error('LAN verification runs from macOS with --peer-workspace and IPv4')
if sys.platform not in ('darwin', 'linux'):
    parser.error('native verification currently supports macOS/Linux')
binary = ROOT / 'target/release/neonmix-hub'
audio = ROOT / 'target/release/neonmix-audio'
out = ROOT / 'artifacts/e02-e04' / time.strftime('%Y%m%d-%H%M%S')
out.mkdir(parents=True)
lab = Path(tempfile.mkdtemp(prefix='hub-probe-', dir=ROOT / '.local/tmp'))
children, handles = [], []
report = {'platform': sys.platform, 'device': args.device, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'passed': False, 'checks': {}}
device_controls = None
original_rate = None
linux_output = sys.platform == 'linux' or bool(args.linux_peer and args.hub_platform == 'linux')
if sys.platform == 'darwin' and not linux_output:
    from macos_audio_controls import DeviceControls
    device_controls = DeviceControls(args.device.removeprefix('coreaudio:'))
    original_rate = device_controls.rate()

peer_lab = None
host = 'localhost'
if args.linux_peer:
    host = args.linux_peer.split('@')[-1] if linux_output else args.macos_host
    report['hub_platform'] = args.hub_platform
    report['hosts'] = {'hub': host, 'linux': args.linux_peer.split('@')[-1], 'macos': args.macos_host}
    peer_lab = str(Path(args.peer_workspace) / '.local/tmp' / lab.name)
    ssh_options = ['-o', f'ControlPath={ROOT}/.local/ssh/ubuntu-112.sock', '-o', f'UserKnownHostsFile={ROOT}/.local/ssh/known_hosts', '-o', 'BatchMode=yes']


def peer_run(argv, **kwargs):
    return subprocess.run(['ssh', *ssh_options, args.linux_peer, shlex.join(argv)], check=True, **kwargs)


class PeerChild:
    """Own one remote child; channel EOF also requests bounded teardown."""
    def __init__(self, argv, stdout, stderr):
        self.args = argv
        mapped = [str(value).replace(str(lab), peer_lab).replace(str(ROOT / 'target/release'), str(Path(args.peer_workspace) / 'target/release')) for value in argv]
        remote = ['runuser', '-u', 'parallels', '--', str(Path(args.peer_workspace) / '.local/audio-env.sh'), 'python3', str(Path(peer_lab) / 'child.py'), *mapped]
        if args.peer_audio_rlimits:
            # runuser resets RTPRIO/NICE limits on current util-linux. Drop UID
            # with setpriv so only these child-process grants survive; no caps.
            remote = ['prlimit', '--rtprio=20:20', '--nice=31:31', '--rttime=200000:200000', '--',
                      'setpriv', '--reuid=parallels', '--regid=parallels', '--init-groups', '--', *remote[4:]]
        self.child = subprocess.Popen(['ssh', *ssh_options, args.linux_peer, shlex.join(remote)], stdin=subprocess.PIPE, stdout=stdout, stderr=stderr)
    def poll(self):
        return self.child.poll()
    def wait(self, timeout=None):
        return self.child.wait(timeout=timeout)
    def send_signal(self, value):
        if self.child.poll() is None:
            try:
                self.child.stdin.write(b'INT\n')
                self.child.stdin.flush()
            except BrokenPipeError:
                pass
    def kill(self):
        self.child.stdin.close()
        self.child.kill()


def start(name, argv):
    stdout, stderr = (out / f'{name}.jsonl').open('w'), (out / f'{name}.stderr').open('w')
    handles.extend([stdout, stderr])
    remote = args.linux_peer and ((argv[1] in ('serve', 'capture') and linux_output) or (argv[1] == 'send' and name == 'sender-b'))
    child = PeerChild(argv, stdout, stderr) if remote else subprocess.Popen(argv, cwd=ROOT, stdout=stdout, stderr=stderr)
    children.append(child)
    return child


def api(path, method='GET', body=None, credential='admin', token=None):
    conn = http.client.HTTPSConnection(host, port, context=context, timeout=5)
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
    mode = ['--mode', 'loopback'] if linux_output else ['--period', '256']
    child = start(name, [str(audio), 'capture', '--device', args.device, '--rate', '48000', '--seconds', str(seconds), *mode])
    deadline = time.monotonic() + seconds + 8
    with (out / f'{name}-diagnostics.jsonl').open('w') as samples:
        while child.poll() is None:
            try:
                child.wait(timeout=min(1, max(.01, deadline - time.monotonic())))
            except subprocess.TimeoutExpired:
                if time.monotonic() >= deadline:
                    raise
                if seconds > 10:
                    samples.write(json.dumps({'at_monotonic': time.monotonic(), 'diagnostics': api('/v1/diagnostics')[1]}) + '\n')
                    samples.flush()
    assert child.wait() == 0
    rows = [json.loads(line) for line in (out / f'{name}.jsonl').read_text().splitlines()]
    final = next(r for r in rows if r['event'] == 'capture_complete')
    stats, measurement = final['stats'], final['measurement']
    report['checks'][name] = final
    assert stats['errors'] == stats['callback_over_budget'] == 0
    if silence:
        assert measurement['peak'] < 1e-7
    else:
        assert measurement['peak'] > .001
        if frequency is not None:
            assert abs(measurement['estimated_frequency_hz'] - frequency) < 1.0
    return measurement


def websocket(after):
    conn = context.wrap_socket(socket.create_connection((host, port), timeout=5), server_hostname=host)
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
    if device_controls is not None:
        device_controls.set_rate(48000)
    extra_hosts = ['--host', host] if args.linux_peer else []
    subprocess.run([str(binary), 'init', '--directory', str(lab), '--output', args.device, *extra_hosts], check=True, stdout=subprocess.DEVNULL)
    if peer_lab:
        # This helper owns only its explicit child, including when SSH disappears.
        (lab / 'child.py').write_text('''import selectors,signal,subprocess,sys
child=subprocess.Popen(sys.argv[1:])
selector=selectors.DefaultSelector();selector.register(sys.stdin,selectors.EVENT_READ)
try:
 while child.poll() is None:
  if selector.select(.1):
   sys.stdin.readline();child.send_signal(signal.SIGINT)
   try:child.wait(timeout=5)
   except subprocess.TimeoutExpired:child.kill();child.wait()
finally:
 if child.poll() is None:child.kill();child.wait()
sys.exit(child.returncode)
''')
        peer_run(['mkdir', '-p', peer_lab])
        for source in list(lab.iterdir()):
            # Only the remote server config uses remote state and credential paths.
            copied = source
            if source.name == 'server.json':
                copied = lab / 'remote-server.json'
                copied.write_text(source.read_text().replace(str(lab), peer_lab))
            subprocess.run(['scp', *ssh_options, str(copied), args.linux_peer + ':' + peer_lab + '/' + source.name], check=True)
        peer_run(['chown', '-R', 'parallels:parallels', peer_lab])
        peer_run(['chmod', '700', peer_lab])
        identity = peer_run(['runuser', '-u', 'parallels', '--', str(Path(args.peer_workspace) / '.local/audio-env.sh'), 'sha256sum', str(Path(args.peer_workspace) / 'target/release/neonmix-hub')], capture_output=True, text=True)
        report['linux_binary_sha256'] = identity.stdout.split()[0]
    credentials = {name: json.loads((lab / f'{name}.json').read_text()) for name in ('admin', 'sender-a', 'sender-b')}
    context = ssl.create_default_context(cadata=credentials['admin']['certificate'])
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    with socket.socket(socket.AF_INET6 if args.ipv6 else socket.AF_INET) as free:
        free.bind(('::1' if args.ipv6 else '127.0.0.1', 0))
        port = free.getsockname()[1]
    listen = f'[::1]:{port}' if args.ipv6 else f'{"0.0.0.0" if args.linux_peer else "127.0.0.1"}:{port}'
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
    sender_seconds = str(max(90, args.soak_seconds + 90))
    sender_a = start('sender-a', [str(binary), 'send', '--credential', str(lab / 'sender-a.json'), '--hub', f'https://{host}:{port}', '--seconds', sender_seconds, '--frequency', '437'])
    state = wait_streams(1)
    a = next(iter(state['streams'].values()))
    if not args.soak_only:
        single = capture('single', 437)
    sender_b = start('sender-b', [str(binary), 'send', '--credential', str(lab / 'sender-b.json'), '--hub', f'https://{host}:{port}', '--seconds', sender_seconds, '--frequency', '659'])
    state = wait_streams(2)
    b = next(s for s in state['streams'].values() if s['id'] != a['id'])
    if not args.soak_only:
        capture('dual')
    if args.soak_seconds:
        capture('dual_soak', seconds=args.soak_seconds)
        diagnostics = api('/v1/diagnostics')[1]
        report['checks']['dual_soak_diagnostics'] = diagnostics
        assert sender_a.poll() is None and sender_b.poll() is None, 'Sender stopped during soak'
        assert len(diagnostics['receivers']) == 2, 'soak requires two active receivers'
        assert diagnostics['underrun_frames'] == 0, diagnostics
        assert diagnostics['output_stats']['errors'] == diagnostics['output_stats']['callback_over_budget'] == 0
        assert all(r['queue_drops'] == 0 and not r['pcm_timing_gaps'] for r in diagnostics['receivers'])
        assert all(r['lost_packets'] == r['plc_samples'] == r['pcm_sink_dropped'] == 0 for r in diagnostics['receivers'])
    if args.soak_only:
        report['passed'] = True
        raise SystemExit(0)  # The common finally block owns teardown/reporting.
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
    saved = json.loads(peer_run(['cat', peer_lab + '/state.json'], capture_output=True, text=True).stdout) if args.linux_peer and linux_output else json.loads((lab / 'state.json').read_text())
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
    report['error'] = repr(error)
    report['traceback'] = traceback.format_exc()
    try:
        report['failure_diagnostics'] = api('/v1/diagnostics')[1]
    except Exception:
        pass
finally:
    for child in sorted(reversed(children), key=lambda child: child.args[1] == 'serve'):
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
    if peer_lab:
        peer_run(['rm', '-rf', peer_lab])
    if device_controls is not None:
        device_controls.set_rate(original_rate)
    (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'evidence': str(out), 'error': report.get('error')}, indent=2))
raise SystemExit(0 if report['passed'] else 1)
