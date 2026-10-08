#!/usr/bin/env python3
"""Full authenticated worker admission/cancel/volume probe; no audio device.

All identities, plugin registries and reports stay inside the project. Uses
signed pair-verify and encrypted classic UDP PCM, not real Apple hardware.
"""
import argparse
import base64
import hashlib
import json
import math
import os
from pathlib import Path
import plistlib
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / 'apps/airplay-worker'), str(ROOT / 'tools')]
import auth_probe
from credential_store_probe import protect_fixture


def ntp(ns):
    return ((ns // 1000000000 + 2208988800) << 32) | ((ns % 1000000000) * (1 << 32) // 1000000000)


class Rig:
    def __init__(self, args):
        self.directory = ROOT / '.local/tmp' / ('session-probe-' + uuid.uuid4().hex)
        self.directory.mkdir(parents=True)
        self.key = self.directory / 'identity.pem'
        subprocess.run([args.openssl, 'genpkey', '-algorithm', 'ED25519', '-out', str(self.key)],
                       check=True, capture_output=True)
        protect_fixture(self.key)
        self.crypto = auth_probe.Crypto(Path(args.crypto).resolve())
        self.identities = [os.urandom(32), os.urandom(32)]
        self.publics = [self.crypto.primitive('probe_ed_public', private) for private in self.identities]
        self.events = []
        self.frames = []
        self.condition = threading.Condition()
        self.write_lock = threading.Lock()
        self.done = threading.Event()
        self.sockets = []
        self.threads = []
        self.pending_cancel = False
        self.listener = self.socket()
        self.listener.listen(1)
        self.listener.settimeout(5)
        env = os.environ.copy()
        env['GST_REGISTRY'] = str(self.directory / 'registry.bin')
        env['NEONMIX_AUDIO_REGISTRY'] = env['GST_REGISTRY']
        env['GST_PLUGIN_SYSTEM_PATH_1_0'] = str(Path(args.plugins).resolve())
        self.process = subprocess.Popen([args.worker], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, env=env)
        self.thread(self.read_events)
        self.command(dict(control_version=2, pcm_version=2, session_control_version=1, worker_generation=7,
            trust_generation=1, pairing_remaining_ms=600000, pairing_attempts=0,
            media_address=f'127.0.0.1:{self.listener.getsockname()[1]}', ipc_token='a' * 64,
            device_id='001122334455', receiver_uuid='00112233-4455-6677-8899-aabbccddeeff',
            keyfile=str(self.key), name='Session probe', pin='1234', rtsp_port=0,
            session_id=1, stream_id=2, stream_epoch=1, format_epoch=1, mapping_id=1,
            known_client_keys=[base64.b64encode(public).decode() for public in self.publics],
            blocked_client_keys=[], pairing_allowed=True))
        self.media, _ = self.listener.accept()
        self.sockets.append(self.media)
        self.media.settimeout(.2)
        self.thread(self.read_media)
        self.ready = self.wait_event('ready', 0)
        assert self.ready['session_control_version'] == 1

    def thread(self, function):
        thread = threading.Thread(target=function, daemon=True)
        self.threads.append(thread)
        thread.start()

    def socket(self, kind=socket.SOCK_STREAM):
        channel = socket.socket(socket.AF_INET, kind)
        channel.bind(('127.0.0.1', 0))
        self.sockets.append(channel)
        return channel

    def command(self, message):
        with self.write_lock:
            self.process.stdin.write(json.dumps(message) + '\n')
            self.process.stdin.flush()

    def read_events(self):
        for line in self.process.stdout:
            event = json.loads(line)
            with self.condition:
                self.events.append(event)
                self.condition.notify_all()
            if event['type'] == 'admit_request':
                if self.pending_cancel:
                    self.cancel(event, 100, 2)
                self.command(dict(type='admit', worker_generation=7, connection_id=event['connection_id'],
                                  request_id=event['request_id'], allowed=True))

    def read_media(self):
        pending = bytearray()
        authenticated = False
        while not self.done.is_set():
            try:
                chunk = self.media.recv(65536)
                if not chunk:
                    return
                pending.extend(chunk)
            except socket.timeout:
                continue
            except OSError:
                return
            if not authenticated:
                if len(pending) < 65:
                    continue
                assert pending[:65] == b'a' * 64 + b'\n'
                del pending[:65]
                authenticated = True
            while len(pending) >= 120:
                assert pending[:4] == b'NMAM'
                length = 120 + struct.unpack_from('<I', pending, 8)[0]
                if len(pending) < length:
                    break
                session, = struct.unpack_from('<Q', pending, 16)
                epoch, = struct.unpack_from('<Q', pending, 32)
                gain, = struct.unpack_from('<f', pending, 96)
                samples = struct.unpack_from('<' + 'f' * ((length - 120) // 4), pending, 120)
                assert all(math.isfinite(sample) for sample in samples)
                with self.condition:
                    self.frames.append(dict(session=session, epoch=epoch, gain=gain,
                                            nonzero=any(abs(sample) > .001 for sample in samples)))
                    self.condition.notify_all()
                del pending[:length]

    def wait_event(self, kind, after, owner=None):
        with self.condition:
            def found():
                return next((e for e in self.events[after:] if e['type'] == kind and
                    (owner is None or (e.get('connection_id'), e.get('request_id')) ==
                     (owner['connection_id'], owner['request_id']))), None)
            assert self.condition.wait_for(lambda: found() is not None, 5), f'event timeout: {kind}'
            return found()

    def cancel(self, owner, session, epoch):
        self.command(dict(type='disconnect', worker_generation=7, connection_id=owner['connection_id'],
                          request_id=owner['request_id'], session_id=session, stream_epoch=epoch))

    def grant(self, owner, session, epoch):
        after = len(self.events)
        self.command(dict(type='grant', worker_generation=7, connection_id=owner['connection_id'],
            request_id=owner['request_id'], session_id=session, stream_id=session + 1,
            stream_epoch=epoch, format_epoch=1, mapping_id=epoch))
        self.wait_event('grant_applied', after, owner)

    def client(self, identity):
        return Client(self, identity)

    def close(self):
        if self.process.poll() is None:
            self.command(dict(type='stop'))
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.done.set()
        for channel in self.sockets:
            channel.close()
        for thread in self.threads:
            thread.join(timeout=2)
        assert not any(e['type'] == 'fatal' for e in self.events), 'worker fatal event'
        assert self.process.returncode == 0, 'worker failed to stop normally'
        shutil.rmtree(self.directory)


class Client:
    def __init__(self, rig, identity):
        self.rig = rig
        self.control = socket.create_connection(('127.0.0.1', rig.ready['port']), timeout=5)
        rig.sockets.append(self.control)
        self.sequence = 0
        self.audio_sequence = 0
        self.secret = auth_probe.pair_verify(self.request, rig.crypto, rig.identities[identity], rig.publics[identity])
        self.fp = bytearray(164)
        self.fp[4] = 3
        assert self.request('POST', '/fp-setup', bytes(self.fp))[0] == 200
        self.timing = rig.socket(socket.SOCK_DGRAM)
        self.timing.settimeout(.2)
        rig.thread(self.timer)
        self.source = rig.socket(socket.SOCK_DGRAM)

    def request(self, method, url, body=b'', content_type='application/x-apple-binary-plist'):
        self.sequence += 1
        self.control.sendall(f'{method} {url} RTSP/1.0\r\nCSeq: {self.sequence}\r\nContent-Type: {content_type}\r\nContent-Length: {len(body)}\r\n\r\n'.encode() + body)
        data = b''
        while b'\r\n\r\n' not in data:
            chunk = self.control.recv(4096)
            if not chunk:
                raise ConnectionError('RTSP closed')
            data += chunk
        header, body = data.split(b'\r\n\r\n', 1)
        length = next((int(line.split(b':', 1)[1]) for line in header.split(b'\r\n')
                       if line.lower().startswith(b'content-length:')), 0)
        while len(body) < length:
            chunk = self.control.recv(4096)
            if not chunk:
                raise ConnectionError('RTSP body closed')
            body += chunk
        return int(header.split(b' ')[1]), body[:length]

    def timer(self):
        while not self.rig.done.is_set():
            try:
                data, address = self.timing.recvfrom(128)
                now = ntp(time.time_ns())
                self.timing.sendto(b'\x80\xd3\x00\x07' + bytes(4) + data[24:32] + struct.pack('>QQ', now, now), address)
            except socket.timeout:
                continue
            except OSError:
                return

    def setup(self):
        after = len(self.rig.events)
        body = plistlib.dumps(dict(eiv=bytes(16), ekey=bytes(72), deviceID='11:22:33:44:55:66',
            name='Synthetic', timingProtocol='NTP', timingPort=self.timing.getsockname()[1]), fmt=plistlib.FMT_BINARY)
        code, response = self.request('SETUP', 'rtsp://receiver/audio', body)
        assert code == 200, f'SETUP status {code}; event counts ' + str({kind:sum(e["type"]==kind for e in self.rig.events) for kind in ['admit_request','session_started','session_ended']})
        self.owner = self.rig.wait_event('session_started', after)
        return self.owner

    def repeat_setup(self):
        # Repeating initial key SETUP is deliberately rejected by the Alpha
        # protocol. Repeat the supported stream SETUP on the same owner.
        self.stream()

    def stream(self):
        body = plistlib.dumps(dict(streams=[dict(type=96, ct=1, spf=352, audioFormat=4,
            controlPort=self.source.getsockname()[1])]), fmt=plistlib.FMT_BINARY)
        code, body = self.request('SETUP', 'rtsp://receiver/audio', body)
        assert code == 200
        self.ports = plistlib.loads(body)['streams'][0]

    def volume(self, volume):
        after = len(self.rig.events)
        assert self.request('SET_PARAMETER', 'rtsp://receiver/audio', f'volume: {volume}\r\n'.encode(), 'text/parameters')[0] == 200
        self.rig.wait_event('volume', after, self.owner)

    def audio(self, session, epoch, expected_gain):
        assert self.request('RECORD', 'rtsp://receiver/audio')[0] == 200
        start = time.time_ns() + 1000000000
        rtp = 50000 + self.audio_sequence * 352
        self.source.sendto(b'\x90\xd4\x00\x04' + struct.pack('>IQI', rtp, ntp(start), rtp + 352), ('127.0.0.1', self.ports['controlPort']))
        pcm = b''.join(struct.pack('>hh', int(6000 * math.sin(i * .05)), int(6000 * math.sin(i * .05))) for i in range(352))
        key = auth_probe.sha(self.rig.crypto.fairplay(bytes(self.fp), bytes(72)) + self.secret)[:16]
        payload = self.rig.crypto.cipher('probe_cbc', pcm, key, bytes(16))
        for index in range(50):
            self.source.sendto(b'\x80\x60' + struct.pack('>HII', (self.audio_sequence+index)&65535, (rtp + index * 352)&0xffffffff, 0) + payload,
                               ('127.0.0.1', self.ports['dataPort']))
            time.sleep(.008)  # Real classic RTP pacing; state barriers use events.
        self.audio_sequence += 50
        with self.rig.condition:
            def frames():
                return [frame for frame in self.rig.frames if frame['session'] == session and frame['epoch'] == epoch]
            assert self.rig.condition.wait_for(lambda: len(frames()) >= 5, 5), 'missing normalized PCM'
            assert all(abs(frame['gain'] - expected_gain) < .00001 for frame in frames()), 'wrong session protocol gain'
            assert any(frame['nonzero'] for frame in frames()), 'fixture did not produce nonzero decoded PCM'
            return len(frames())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--worker', required=True)
    parser.add_argument('--crypto', required=True)
    parser.add_argument('--plugins', required=True)
    parser.add_argument('--openssl', required=True)
    parser.add_argument('--report', required=True)
    args = parser.parse_args()
    report = Path(args.report).resolve()
    report.relative_to(ROOT)
    report.parent.mkdir(parents=True, exist_ok=True)
    results = []
    def save():
        report.write_text(json.dumps(dict(passed=False,results=results,scope='in-progress full worker probe'),indent=2)+'\n')
    for stage in ['admission_pending', 'await_grant', 'active']:
        rig = Rig(args)
        try:
            rig.pending_cancel = stage == 'admission_pending'
            client = rig.client(0)
            after = len(rig.events)
            try:
                owner = client.setup()
                if stage == 'active':
                    client.stream(); rig.grant(owner, 100, 2); client.audio(100, 2, 1.)
                rig.cancel(owner, 100, 2)
            except (ConnectionError, AssertionError):
                if stage != 'admission_pending':
                    raise
                owner = rig.wait_event('admit_request', after)
            rig.wait_event('session_ended', after, owner)
            rig.pending_cancel = False
            count = sum(e['type'] == 'grant_applied' for e in rig.events)
            rig.command(dict(type='grant', worker_generation=7, connection_id=owner['connection_id'], request_id=owner['request_id'],
                session_id=100, stream_id=101, stream_epoch=2, format_epoch=1, mapping_id=2))
            successor = rig.client(1); new_owner = successor.setup(); successor.stream(); rig.grant(new_owner, 200, 2)
            blocks = successor.audio(200, 2, 1.)
            assert sum(e['type'] == 'grant_applied' for e in rig.events) == count + 1, 'late grant reopened old owner'
            results.append(dict(case=stage, passed=True, successor_pcm_blocks=blocks))
            save()
        finally:
            rig.close()
    for volume in [-144., -18., 0.]:
        rig = Rig(args)
        try:
            source = rig.client(0); owner = source.setup(); source.stream(); rig.grant(owner, 100, 2)
            source.volume(volume)
            setup_after = len(rig.events)
            source.repeat_setup()
            rig.wait_event('flush', setup_after, owner); rig.grant(owner, 100, 3)
            legal = 0. if volume <= -144 else 10 ** (volume / 20)
            a_blocks = source.audio(100, 3, legal)
            after = len(rig.events)
            assert source.request('FLUSH', 'rtsp://receiver/audio')[0] == 200
            rig.wait_event('flush', after, owner); rig.grant(owner, 100, 4)
            resumed = source.audio(100, 4, legal)
            rig.cancel(owner, 100, 4); rig.wait_event('session_ended', after, owner)
            successor = rig.client(1); new_owner = successor.setup(); successor.stream(); rig.grant(new_owner, 200, 2)
            b_blocks = successor.audio(200, 2, 1.)
            results.append(dict(case='volume', volume_db=volume, passed=True, first_pcm_blocks=a_blocks,
                                resumed_pcm_blocks=resumed, successor_pcm_blocks=b_blocks, repeated_stream_setup=True))
            save()
        finally:
            rig.close()
    data = dict(passed=True, worker_sha256=hashlib.sha256(Path(args.worker).read_bytes()).hexdigest(),
                results=results, scope='signed synthetic sender, encrypted UDP, actual worker and PCM IPC; no Apple/hardware audio')
    report.write_text(json.dumps(data, indent=2) + '\n')
    print(json.dumps(data, indent=2))


if __name__ == '__main__':
    main()
