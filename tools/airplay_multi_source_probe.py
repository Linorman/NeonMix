#!/usr/bin/env python3
"""Isolated macOS multi-AirPlay digital source acceptance against /v2/airplay.

Requires an explicitly selected real CoreAudio output and already-built Hub,
worker and crypto probe. Creates a fresh project-local profile; never attaches
an existing room. Uses synthetic FairPlay, SRP and encrypted PCM, not Apple
hardware. No audio, bearer tokens, PINs or receiver/source keys enter the report.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import plistlib
import re
import signal
import socket
import ssl
import struct
import subprocess
import sys
import threading
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen
import uuid

from airplay_hub_probe import child_workers, wait_for
from airplay_probe_fixtures import ALAC
import shutil

ROOT = Path(__file__).resolve().parents[1]
DEV = ROOT / 'tools/dev'
sys.path.insert(0, str(ROOT / 'apps/airplay-worker'))
from auth_probe import Crypto, pin_setup, pair_verify, sha


def assert_continuity(window, phase):
    """Fail on aggregate skips as well as surviving-source deadline failures."""
    assert window['global_timed_late_frames_delta'] == 0, f'{phase} Mixer skipped timed PCM'
    for row in window['lanes']:
        assert row['underrun_frames_delta'] == 0, f'{phase} Mixer lane underrun'
        if row['kind'] == 'airplay':
            assert row['late_packets_delta'] == 0 and not row['counter_reset'], f'{phase} AirPlay ingress lost deadlines'


def ntp(ns):
    return (((ns // 1_000_000_000) + 2208988800) << 32) | ((ns % 1_000_000_000) * (1 << 32) // 1_000_000_000)


class DigitalSource:
    """One independent identity, RTSP socket, NTP clock and encrypted RTP source."""
    def __init__(self, index, crypto_library=None, codec="pcm", omit_spf=False):
        self.index = index
        self.codec = codec
        self.omit_spf = omit_spf
        self.private = os.urandom(32)
        self.crypto = Crypto(crypto_library)
        self.public = self.crypto.primitive('probe_ed_public', self.private)
        self.frequency = [317, 439, 563, 701][index]
        self.name = f'NeonMix isolated digital source {index + 1}'
        self.sockets = []
        self.threads = []
        self.stop = threading.Event()
        self.errors = []
        self.paired = False
        self.sequence = 0
        self.sent_packets = 0
        self.max_schedule_lag_ns = 0
        self.max_send_gap_ns = 0
        self.protocol_stage = 'created'
        self.last_response = None

    def udp(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(('127.0.0.1', 0))
        self.sockets.append(sock)
        return sock

    def rtsp(self, method, path, body=b''):
        self.sequence += 1
        self.last_response = dict(method=method, route=path, status=None, response_bytes=0)
        content = 'Content-Type: application/x-apple-binary-plist\r\n' if body else ''
        self.control.sendall(f'{method} {path} RTSP/1.0\r\nCSeq: {self.sequence}\r\nContent-Length: {len(body)}\r\n{content}\r\n'.encode() + body)
        reply = b''
        while b'\r\n\r\n' not in reply:
            chunk = self.control.recv(4096)
            assert chunk, 'RTSP closed before response'
            reply += chunk
            assert len(reply) <= 32768, 'RTSP response exceeds probe bound'
        header, response = reply.split(b'\r\n\r\n', 1)
        length = next((int(line.split(b':', 1)[1]) for line in header.split(b'\r\n')[1:]
                       if line.lower().startswith(b'content-length:')), 0)
        assert 0 <= length <= 16384, 'RTSP body exceeds probe bound'
        while len(response) < length:
            chunk = self.control.recv(length - len(response))
            assert chunk, 'RTSP response truncated'
            response += chunk
        status = int(header.split(b' ')[1])
        self.last_response.update(status=status, response_bytes=length)
        return status, response[:length]

    def start(self, port, pin, admission_barrier=None, allow_rejected=False):
        self.stop.clear()
        self.errors.clear()
        self.sequence = 0
        self.control = socket.create_connection(('127.0.0.1', port), timeout=4)
        self.sockets.append(self.control)
        self.protocol_stage = 'receiver_info'
        code, body = self.rtsp('GET', '/info')
        assert code == 200, 'receiver info rejected'
        receiver_key = plistlib.loads(body)['pk']
        assert len(receiver_key) == 32, 'receiver public key missing'
        if not self.paired:
            self.protocol_stage = 'pin_setup'
            assert self.rtsp('POST', '/pair-pin-start')[0] == 200, 'PIN start rejected'
            assert pin_setup(self.rtsp, self.crypto, pin, self.private, proof_bytes=20,
                             expected_pin=pin, expected_server_public=receiver_key) == self.public
            self.paired = True
        self.protocol_stage = 'pair_verify'
        secret = pair_verify(self.rtsp, self.crypto, self.private, self.public,
                             expected_server_public=receiver_key)
        self.protocol_stage = 'fairplay'
        fp = bytearray(164)
        fp[4] = 3
        assert self.rtsp('POST', '/fp-setup', bytes(fp))[0] == 200, 'FairPlay fixture rejected'
        timing = self.udp()
        timing.settimeout(.2)
        timing_seen = threading.Event()

        def reply_timing():
            try:
                while not self.stop.is_set():
                    try:
                        data, peer = timing.recvfrom(128)
                    except socket.timeout:
                        continue
                    assert len(data) >= 32, 'short NTP request'
                    now = ntp(time.time_ns())
                    timing.sendto(b'\x80\xd3\x00\x07' + bytes(4) + data[24:32] + struct.pack('>QQ', now, now), peer)
                    timing_seen.set()
            except Exception as error:
                if not self.stop.is_set():
                    self.errors.append(type(error).__name__)

        thread = threading.Thread(target=reply_timing, daemon=True)
        self.threads.append(thread)
        thread.start()
        root = lambda value: plistlib.dumps(value, fmt=plistlib.FMT_BINARY)
        setup = dict(eiv=bytes(16), ekey=bytes(72), deviceID='11:22:33:44:55:66', name=self.name,
                     timingProtocol='NTP', timingPort=timing.getsockname()[1])
        if admission_barrier:
            admission_barrier.wait(timeout=10)
        self.protocol_stage = 'initial_setup'
        self.admission_status = self.rtsp('SETUP', 'rtsp://receiver/audio', root(setup))[0]
        if allow_rejected and self.admission_status != 200:
            return receiver_key
        assert self.admission_status == 200, 'Hub audio admission rejected'
        assert timing_seen.wait(3), 'NTP exchange missing'
        audio = self.udp()
        self.protocol_stage = 'stream_setup'
        stream = dict(type=96, ct=2 if self.codec == 'alac' else 1, audioFormat=262144 if self.codec == 'alac' else 4, controlPort=audio.getsockname()[1])
        if not self.omit_spf:
            stream['spf'] = 352
        code, body = self.rtsp('SETUP', 'rtsp://receiver/audio', root(dict(streams=[stream])))
        assert code == 200, 'PCM stream setup rejected'
        ports = plistlib.loads(body)['streams'][0]
        key = sha(self.crypto.fairplay(bytes(fp), bytes(72)) + secret)[:16]
        origin = 0xffffff00
        target = time.time_ns() + 2_000_000_000
        audio.sendto(b'\x90\xd4\x00\x04' + struct.pack('>IQI', origin, ntp(target), (origin + 352) & 0xffffffff),
                     ('127.0.0.1', ports['controlPort']))
        time.sleep(.05)

        def send_audio():
            began = time.monotonic()
            began_ns = time.monotonic_ns()
            previous_send_ns = None
            index = 0
            try:
                while not self.stop.is_set():
                    position = index * 352
                    pcm = ALAC if self.codec == 'alac' else b''.join(struct.pack('<hh', *([int(.08 * 32767 * math.sin(
                        2 * math.pi * self.frequency * (position + frame) / 44100))] * 2)) for frame in range(352))
                    block_bytes = len(pcm) // 16 * 16
                    encrypted = self.crypto.cipher('probe_cbc', pcm[:block_bytes], key, bytes(16)) + pcm[block_bytes:]
                    sending_ns = time.monotonic_ns()
                    scheduled_ns = began_ns + index * 352 * 1_000_000_000 // 44100
                    self.max_schedule_lag_ns = max(self.max_schedule_lag_ns, sending_ns - scheduled_ns)
                    if previous_send_ns is not None:
                        self.max_send_gap_ns = max(self.max_send_gap_ns, sending_ns - previous_send_ns)
                    previous_send_ns = sending_ns
                    audio.sendto(b'\x80\x60' + struct.pack('>HII', index & 0xffff, (origin + position) & 0xffffffff, 0) + encrypted,
                                 ('127.0.0.1', ports['dataPort']))
                    index += 1
                    self.sent_packets += 1
                    self.stop.wait(max(0, began + index * 352 / 44100 - time.monotonic()))
            except Exception as error:
                if not self.stop.is_set():
                    self.errors.append(type(error).__name__)

        thread = threading.Thread(target=send_audio, daemon=True)
        self.threads.append(thread)
        thread.start()
        self.protocol_stage = 'sending'
        return receiver_key

    def close(self):
        self.stop.set()
        for thread in self.threads:
            thread.join(timeout=2)
        for sock in self.sockets:
            sock.close()
        self.threads.clear()
        self.sockets.clear()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scenario', choices=['mix', 'management', 'mix-control', 'shutdown'], default='mix', help='Management and mix-control run focused two-source checks without repeating the mix/fault matrix')
    parser.add_argument('--profile', choices=['debug', 'release'], default='release')
    parser.add_argument('--sources', type=int, choices=range(5), default=2)
    parser.add_argument('--native-sources', type=int, choices=range(5), default=0, help='Independent native tone Senders; total inputs must be 1..8')
    parser.add_argument('--output', required=True, help='Explicit CoreAudio UID, for example coreaudio:BlackHole2ch_UID')
    parser.add_argument('--fault-cycles', type=int, choices=range(1, 21), default=1, help='Repeat every AirPlay disconnect/crash/recovery sequence this many times')
    parser.add_argument('--shutdown-active', action='store_true', help='Stop the managed Hub while all synthetic sources remain active and verify worker/key cleanup')
    parser.add_argument('--steady-seconds', type=float, default=3)
    parser.add_argument('--status-readers', type=int, choices=range(9), default=0, help='Concurrent status readers exercising admission during profile-lock contention')
    parser.add_argument('--report', type=Path, help='Project-local JSON report; defaults to artifacts/airplay/multi-source/<unique-run>/result.json')
    args = parser.parse_args()
    args.shutdown_active = args.shutdown_active or args.scenario == "shutdown"
    assert args.scenario not in ['management', 'mix-control'] or (args.sources == 2 and args.native_sources == 0 and args.fault_cycles == 1), 'management/mix-control requires --sources 2 --native-sources 0 --fault-cycles 1'
    assert platform.system() == 'Darwin', 'macOS-only probe'
    assert 1 <= args.sources + args.native_sources <= 8, 'combined source count must be 1..8'
    assert .5 <= args.steady_seconds <= 60, 'steady duration must be 0.5..60 seconds'
    assert args.output.startswith('coreaudio:') and len(args.output) > len('coreaudio:'), 'explicit CoreAudio output required'
    source_binary = ROOT / 'target' / args.profile / 'neonmix-hub'
    source_worker = source_binary.with_name('neonmix-airplay-worker')
    crypto = ROOT / '.local/airplay/build/libneonmix-airplay-probe-crypto.dylib'
    assert all(path.is_file() for path in [source_binary, source_worker, crypto]), 'build Hub, worker and crypto probe first'
    run = uuid.uuid4().hex[:12]
    report_path = (args.report or ROOT / 'artifacts/airplay/multi-source' / run / 'result.json').resolve()
    report_path.relative_to(ROOT)
    base = ROOT / '.local/airplay-multi' / run
    base.mkdir(parents=True, mode=0o700)
    binary = base / 'bin/neonmix-hub'
    worker = binary.with_name('neonmix-airplay-worker')
    state = base / 'profile'
    state.mkdir(mode=0o700)
    report = dict(scenario=args.scenario, platform='macOS', os_version=platform.mac_ver()[0], architecture=platform.machine(), build_profile=args.profile, sources=args.sources, native_sources=args.native_sources, output_device=args.output,
                  scope='isolated profile; independent encrypted digital AirPlay sources through one Hub and real CoreAudio output',
                  limitations=['synthetic FairPlay; no Apple sender interoperability claim', 'meter/counter verification, no recorded or analog audio',
                               'short functional run, not sustained performance acceptance'],
                  binary_sha256={}, artifact_snapshot=True, fault_cycles=args.fault_cycles,
                  patch_sha256=hashlib.sha256((ROOT / 'patches/airplay-audio-only.patch').read_bytes()).hexdigest(),
                  steady_seconds=None if args.scenario != 'mix' else (max(11, args.steady_seconds) if args.sources else args.steady_seconds),
                  airplay_frequencies_hz=[317, 439, 563, 701][:args.sources],
                  native_frequencies_hz=[809 + 137 * index for index in range(args.native_sources)],
                  passed=False, scenarios={}, isolation_checks=[])
    hub = None
    log_handle = None
    api = None
    sources = []
    extra_sources = []
    native_processes = []
    native_logs = []
    native_streams = []
    readers_stop = threading.Event()
    readers = []
    reader_counts = [0] * args.status_readers
    reader_errors = [0] * args.status_readers
    phase = 'initialization'
    try:
        binary.parent.mkdir(mode=0o700)
        shutil.copy2(source_binary, binary)
        shutil.copy2(source_worker, worker)
        report['binary_sha256'] = {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in [binary, worker]}
        initialized = subprocess.run([str(DEV), str(binary), 'init', '--directory', str(state), '--output', args.output],
                                     capture_output=True, timeout=30, cwd=ROOT)
        assert initialized.returncode == 0, 'isolated profile initialization failed'
        credential = json.loads((state / 'admin.json').read_text())
        # Explicit laboratory init credentials only, before this fresh Hub starts.
        config_path = state / 'server.json'
        config = json.loads(config_path.read_text())
        for index in range(5):
            token = uuid.uuid4().hex + uuid.uuid4().hex
            config['devices'].append(dict(name=f'native-{index + 1}', role='member', token=token))
            path = state / f'native-{index + 1}.json'
            path.write_text(json.dumps(dict(token=token, certificate=credential['certificate'])))
            path.chmod(0o600)
        config_path.write_text(json.dumps(config))
        config_path.chmod(0o600)
        tls = ssl.create_default_context(cadata=credential['certificate'])
        tls.minimum_version = tls.maximum_version = ssl.TLSVersion.TLSv1_3
        log = base / 'hub.log'
        log_handle = log.open('w')
        env = dict(os.environ, NEONMIX_AIRPLAY_RUNTIME=str(ROOT / '.local/airplay'))
        hub = subprocess.Popen([str(DEV), str(binary), 'serve', '--config', str(state / 'server.json'), '--listen', '127.0.0.1:0'] + (['--managed-control-stdin'] if args.shutdown_active else []),
                               stdin=subprocess.PIPE if args.shutdown_active else subprocess.DEVNULL, stdout=log_handle, stderr=log_handle, cwd=ROOT, env=env)

        def started():
            assert hub.poll() is None, 'fixture Hub exited during startup'
            for line in log.read_text().splitlines():
                try:
                    event = json.loads(line)
                except ValueError:
                    continue
                if event.get('event') == 'hub_started':
                    return event['listen']

        url = 'https://' + wait_for('isolated Hub listening', started)

        def request_api(path, body=None):
            request = Request(url + path, data=None if body is None else json.dumps(body).encode(),
                              headers={'Authorization': 'Bearer ' + credential['token'], 'Content-Type': 'application/json'})
            try:
                with urlopen(request, context=tls, timeout=4) as response:
                    return response.status, json.load(response)
            except HTTPError as error:
                return error.code, json.load(error)

        api = request_api

        def get(path):
            code, body = api(path)
            assert code == 200, f'authenticated fixture API read rejected: {path} status={code} error=' + (body.get('error', 'unknown') if re.fullmatch('[a-z_]{1,64}', str(body.get('error', ''))) else 'unknown')
            return body

        def airplay():
            return get('/v2/airplay')

        def diagnostics():
            return get('/v1/diagnostics')

        def command(action, **fields):
            # Retry only revision races, never silently turn a rejected action
            # into another command or duplicate a successfully accepted request.
            for _ in range(10):
                body = dict(command_id=str(uuid.uuid4()), expected_revision=airplay()['revision'],
                            operation=dict(action=action, **fields))
                code, result = api('/v2/airplay', body)
                if code == 409 and result.get('error') == 'stale_revision':
                    continue
                assert code == 200, 'v2 fixture operation rejected: ' + action + ' status=' + str(code) + ' error=' + (result.get('error', 'unknown') if re.fullmatch('[a-z_]{1,64}', str(result.get('error', ''))) else 'unknown')
                return result
            raise AssertionError('v2 revision never settled')

        def continuity_delta(before_snapshot, before_d, after_snapshot, after_d, airplay_indices):
            rows = []
            after_sessions = {session['receiver_id']: session for session in after_snapshot['sessions']}
            before_sessions = {session['receiver_id']: session for session in before_snapshot['sessions']}
            for index in airplay_indices:
                old = before_sessions[receiver_ids[index]]
                current = after_sessions.get(receiver_ids[index])
                lane = old['lane']
                old_late = old['ingress']['late_packets']
                new_late = current['ingress']['late_packets'] if current else None
                rows.append(dict(kind='airplay', source_index=index + 1, lane=lane,
                    underrun_frames_before=before_d['underrun_frames_by_lane'][lane],
                    underrun_frames_after=after_d['underrun_frames_by_lane'][lane],
                    underrun_frames_delta=after_d['underrun_frames_by_lane'][lane] - before_d['underrun_frames_by_lane'][lane],
                    late_packets_before=old_late, late_packets_after=new_late,
                    late_packets_delta=new_late - old_late if new_late is not None and new_late >= old_late else None,
                    counter_reset=new_late is not None and new_late < old_late))
            for index, stream_id in enumerate(native_streams):
                lane = next(i for i, value in enumerate(before_d['meters']['lanes']) if value['stream_id'] == stream_id)
                rows.append(dict(kind='native', source_index=index + 1, lane=lane,
                    underrun_frames_before=before_d['underrun_frames_by_lane'][lane],
                    underrun_frames_after=after_d['underrun_frames_by_lane'][lane],
                    underrun_frames_delta=after_d['underrun_frames_by_lane'][lane] - before_d['underrun_frames_by_lane'][lane]))
            return dict(timed_late_frames_by_lane_delta=[b - a for a, b in zip(before_d['timed_late_frames_by_lane'], after_d['timed_late_frames_by_lane'])],
                last_timed_late={field: after_d[field] for field in ('last_timed_late_stream_id', 'last_timed_late_epoch', 'last_timed_late_output_frame', 'last_timed_late_target_ns', 'last_timed_late_presentation_ns')},
                output_frames=after_d['output_frames'] - before_d['output_frames'],
                global_timed_late_frames_before=before_d['timed_late_frames'],
                global_timed_late_frames_after=after_d['timed_late_frames'],
                global_timed_late_frames_delta=after_d['timed_late_frames'] - before_d['timed_late_frames'],
                global_late_attribution='Mixer aggregate; cannot attribute solely to survivors', lanes=rows)

        def receiver(receiver_id):
            return next(item for item in airplay()['receivers'] if item['receiver_id'] == receiver_id)

        pairing_window_tested = False

        def enable(receiver_id):
            nonlocal pairing_window_tested
            before = set(child_workers(hub))
            command('enable_receiver', receiver_id=receiver_id)
            ready = wait_for('targeted worker ready', lambda: (s if s['ready'] else None) if (s := receiver(receiver_id)) else None, seconds=25)
            added = wait_for('targeted worker PID', lambda: set(child_workers(hub)) - before)
            assert len(added) == 1, 'worker startup did not create exactly one owned child'
            pid = added.pop()
            listing = subprocess.run(['/usr/sbin/lsof', '-a', '-p', str(pid), '-iTCP', '-sTCP:LISTEN', '-Fn'],
                                     capture_output=True, text=True, check=True)
            ports = {int(match[1]) for line in listing.stdout.splitlines()
                     if line.startswith('n') and (match := re.search(r':(\d+)$', line))}
            assert len(ports) == 1, 'expected one RTSP port per owned worker'
            if not pairing_window_tested:
                command('pair_receiver', receiver_id=receiver_id)
                ready = wait_for('new PIN window applied without worker restart', lambda: (r if r['ready'] and r.get('pairing_pin')
                    and r['pairing_window_open'] else None) if (r := receiver(receiver_id)) else None)
                assert set(child_workers(hub)) == before | {pid}, 'PIN window restart replaced a worker'
                pairing_window_tested = True
                report['scenarios']['explicit_idle_pairing_window_renews_without_worker_restart'] = True
            return ready, pid, ports.pop()

        def live(expected_indices):
            assert hub.poll() is None, 'fixture Hub stopped'
            assert all(not sources[i].errors for i in expected_indices), 'digital source thread failed'
            assert all(process.poll() is None for process in native_processes), 'native tone source exited'
            snapshot = airplay()
            sessions = {s['receiver_id']: s for s in snapshot['sessions']}
            d = diagnostics()
            report['last_live_observation'] = dict(expected_sources=list(expected_indices),
                active=snapshot['capacity']['active'],
                sessions=[dict(lane=s['lane'], released_blocks=s['released_blocks'],
                    ingress_late=s['ingress']['late_packets'], lane_rms=d['meters']['lanes'][s['lane']]['rms'])
                    for s in snapshot['sessions']],
                output_rms=d['meters']['output']['rms'], output_stats=d['output_stats'])
            for stream_id in native_streams:
                lane = next((lane for lane in d['meters']['lanes'] if lane['stream_id'] == stream_id), None)
                if not lane or lane['rms'] is None or lane['rms'] < .0002:
                    return None
            for index in expected_indices:
                session = sessions.get(receiver_ids[index])
                if not session or session['released_blocks'] < 20 or session['ingress']['accepted_packets'] < 20:
                    return None
                assert session['ingress']['identity_rejections'] == 0, 'media identity mismatch'
                assert session['ingress']['last_source_rate'] == 44100, 'source clock rate lost'
                if d['meters']['lanes'][session['lane']]['rms'] is None or d['meters']['lanes'][session['lane']]['rms'] < .001:
                    return None
            if d['meters']['output']['rms'] is None or d['meters']['output']['rms'] < .0001:
                return None
            return snapshot, d

        wait_for('CoreAudio running', lambda: get('/v1/hub')['output']['available'] and diagnostics()['output_frames'] > 4800)
        command('configure', receiver_count=3 if args.scenario == 'management' else max(1, args.sources), multi_receiver=True)
        for index in range(args.native_sources):
            path = base / f'native-{index + 1}.log'
            handle = path.open('w')
            native_logs.append(handle)
            process = subprocess.Popen([str(DEV), str(binary), 'send', '--credential', str(state / f'native-{index + 1}.json'),
                '--hub', url, '--seconds', str(max(600, args.fault_cycles * 60 + 120)), '--frequency', str(809 + 137 * index)], stdout=handle, stderr=handle, cwd=ROOT, env=env)
            native_processes.append(process)
            def native_started():
                assert process.poll() is None, 'native tone source exited during startup'
                for line in path.read_text().splitlines():
                    try:
                        event = json.loads(line)
                    except ValueError:
                        continue
                    if event.get('event') == 'sender_started':
                        return event['stream_id']
            native_streams.append(wait_for('independent native tone source starts', native_started))
        receiver_ids = [r['receiver_id'] for r in airplay()['receivers']][:args.sources]
        assert len(receiver_ids) == args.sources, 'configured receiver count differs'
        sources = [DigitalSource(index) for index in range(args.sources)]
        assert len({source.public for source in sources}) == args.sources, 'source identities collided'
        pids = []
        keys = []
        spare = None
        if args.scenario == 'management':
            phase = 'prebind source A to another idle receiver'
            spare_id = airplay()['receivers'][2]['receiver_id']
            ready, spare_pid, spare_port = enable(spare_id)
            prebound = DigitalSource(0)
            prebound.private, prebound.public = sources[0].private, sources[0].public
            extra_sources.append(prebound)
            spare_key = prebound.start(spare_port, ready['pairing_pin'])
            wait_for('source A binding committed at spare receiver', lambda: any(s['receiver_id'] == spare_id for s in airplay()['sessions']))
            prebound.close()
            wait_for('spare receiver idle after prebinding', lambda: not any(s['receiver_id'] == spare_id for s in airplay()['sessions']))
            spare = (spare_id, spare_pid, spare_port, spare_key)
        if args.sources >= 2 and args.scenario == 'mix':
            phase = 'same source simultaneous cross-receiver admission'
            race_sources = [DigitalSource(0), DigitalSource(0)]
            race_sources[1].private = race_sources[0].private
            race_sources[1].public = race_sources[0].public
            race_workers = [enable(receiver_id) for receiver_id in receiver_ids[:2]]
            barrier = threading.Barrier(2)
            race_errors = []
            def race(index):
                try:
                    race_sources[index].start(race_workers[index][2], race_workers[index][0]['pairing_pin'], barrier, True)
                except Exception as error:
                    race_errors.append(type(error).__name__)
            race_threads = [threading.Thread(target=race, args=(index,), daemon=True) for index in range(2)]
            try:
                for thread in race_threads:
                    thread.start()
                for thread in race_threads:
                    thread.join(timeout=15)
                assert not any(thread.is_alive() for thread in race_threads) and not race_errors, 'cross-receiver race setup failed'
                assert sorted(source.admission_status for source in race_sources) == [200, 403], 'same source acquired multiple concurrent receivers'
                wait_for('single admitted owner after cross-receiver race', lambda: len(airplay()['sessions']) == 1)
                report['scenarios']['same_source_simultaneous_cross_receiver_has_one_owner'] = True
            finally:
                for source in race_sources:
                    source.close()
                for receiver_id in receiver_ids[:2]:
                    command('disable_receiver', receiver_id=receiver_id)
                wait_for('competition fixture workers exit', lambda: all(pid not in child_workers(hub) for _, pid, _ in race_workers))
                time.sleep(.2)
        phase = 'independent receiver pairing and encrypted PCM'
        for index, receiver_id in enumerate(receiver_ids):
            ready, pid, port = enable(receiver_id)
            pids.append(pid)
            keys.append(sources[index].start(port, ready['pairing_pin']))
            wait_for('new digital source rendered', lambda i=index: live(range(i + 1)), seconds=15)
        assert len(set(keys)) == args.sources, 'receiver identities were shared'
        first, first_d = wait_for('all digital sources rendered', lambda: live(range(args.sources)))
        def read_status(index):
            while not readers_stop.is_set():
                try:
                    code, _ = api('/v2/airplay')
                    reader_counts[index] += int(code == 200)
                    reader_errors[index] += int(code != 200)
                except Exception:
                    reader_errors[index] += 1
                readers_stop.wait(.005)
        for index in range(args.status_readers):
            thread = threading.Thread(target=read_status, args=(index,), daemon=True)
            readers.append(thread)
            thread.start()
        source_ids = {session['receiver_id']: session['source_id'] for session in first['sessions']}
        assert len(set(source_ids.values())) == args.sources, 'independent sources collapsed into one identity'
        assert len({s['lane'] for s in first['sessions']}) == args.sources, 'concurrent sources share a Mixer lane'
        assert first['capacity']['active'] == args.sources and first['capacity']['reserved'] == 0, 'capacity counters disagree'
        if args.scenario == 'shutdown':
            report['scope']='managed stop and resource cleanup with synthetic PCM; audio timing quality is reported separately'
            report['quality_evaluated']=False
            report['observed_quality_before_stop']=dict(timed_late_frames=first_d['timed_late_frames'],output_stats=first_d['output_stats'],
                ingress=[session['ingress'] for session in first['sessions']])
        elif args.scenario == 'management':
            phase = 'management baseline'
            initial_sessions = {session['receiver_id']: session for session in first['sessions']}
            a_session, b_session = initial_sessions[receiver_ids[0]], initial_sessions[receiver_ids[1]]
            assert 'stream_epoch' in b_session, 'v2 session must expose stream_epoch for exact isolation verification'
            report['management_checks'] = []

            def unchanged(indices, baseline=initial_sessions):
                value = live(indices)
                if not value:
                    return None
                snapshot, d = value
                current = {session['receiver_id']: session for session in snapshot['sessions']}
                for index in indices:
                    session = current[receiver_ids[index]]
                    old = baseline[receiver_ids[index]]
                    for field in ['session_id', 'stream_id', 'stream_epoch']:
                        assert session[field] == old[field], 'management command changed another active context'
                    assert session['ingress']['last_mapping_id'] == old['ingress']['last_mapping_id'], 'management command changed another mapping'
                    if session['released_blocks'] <= old['released_blocks']:
                        return None
                return snapshot, d

            phase = 'live configuration adds and removes an idle receiver'
            before, before_d = wait_for('both sources before live configuration', lambda: unchanged([0, 1]))
            command('configure', receiver_count=4, multi_receiver=True)
            added_id = airplay()['receivers'][3]['receiver_id']
            wait_for('added idle receiver starts', lambda: receiver(added_id)['ready'])
            command('configure', receiver_count=3, multi_receiver=True)
            wait_for('removed idle receiver stops', lambda: not receiver(added_id)['enabled'] and not receiver(added_id)['configured_enabled'])
            after, after_d = wait_for('both original sessions survive live configuration', lambda: unchanged([0, 1]))
            report['management_checks'].append(dict(operation='live_configure_add_remove_idle',
                continuity=continuity_delta(before, before_d, after, after_d, [0, 1]), original_contexts_unchanged=True))
            code, legacy = api('/v1/airplay')
            assert code == 426 and legacy.get('error') == 'upgrade_required', 'legacy v1 did not reject multi-receiver access'
            report['scenarios']['live_configure_idle_entries_preserves_active_sessions'] = True
            report['scenarios']['multi_receiver_v1_requires_upgrade'] = True

            phase = 'revoke source A while source B continues'
            before, before_d = wait_for('both sources before revoke', lambda: unchanged([0, 1]))
            command('revoke_source', source_id=a_session['source_id'], session_id=a_session['session_id'])
            sources[0].close()
            wait_for('revoked session released', lambda: not any(s['source_id'] == a_session['source_id'] for s in airplay()['sessions']))
            after, after_d = wait_for('source B continues after revoke', lambda: unchanged([1]))
            report['management_checks'].append(dict(operation='revoke_source_A', source_B_context_unchanged=True,
                continuity=continuity_delta(before, before_d, after, after_d, [1])))

            def old_key_denied(port):
                attempt = DigitalSource(0)
                attempt.private, attempt.public = sources[0].private, sources[0].public
                extra_sources.append(attempt)
                try:
                    attempt.control = socket.create_connection(('127.0.0.1', port), timeout=4)
                    attempt.sockets.append(attempt.control)
                    xpub = attempt.crypto.primitive('probe_x_public', os.urandom(32))
                    code, body = attempt.rtsp('POST', '/pair-verify', bytes([1, 0, 0, 0]) + xpub + attempt.public)
                    assert code in [200, 403], 'unexpected old-key verification response'
                    return dict(status=code, response_bytes=len(body)) if code == 403 else None
                finally:
                    attempt.close()

            phase = 'global revoke rejects source A old key at previously bound spare receiver'
            denied = wait_for('old cross-receiver binding rejected', lambda: old_key_denied(spare[2]))
            report['management_checks'].append(dict(operation='revoked_key_at_other_idle_receiver', **denied))
            rejection = dict(command_id=str(uuid.uuid4()), expected_revision=airplay()['revision'],
                             operation=dict(action='allow_source', source_id=a_session['source_id']))
            code, refused = api('/v2/airplay', rejection)
            assert code == 400 and refused.get('error') == 'pairing_revoked', 'AllowSource restored a revoked identity'
            report['scenarios']['revoke_global_and_allow_cannot_restore_pairing'] = True

            phase = 'repair source A with a new explicit PIN and independent synchronized mode'
            command('playback_mode', source_id=a_session['source_id'], mode='synchronized')
            command('repair_source', source_id=a_session['source_id'], receiver_id=receiver_ids[0])
            repaired = wait_for('repair PIN window acknowledged', lambda: (r if r.get('pairing_pin') and r['pairing_window_open'] else None)
                                if (r := receiver(receiver_ids[0])) else None)
            # Repair must not make the old signed verification sufficient.
            port = next(iter({int(match[1]) for line in subprocess.run(['/usr/sbin/lsof', '-a', '-p', str(pids[0]), '-iTCP', '-sTCP:LISTEN', '-Fn'],
                capture_output=True, text=True, check=True).stdout.splitlines() if line.startswith('n') and (match := re.search(r':(\d+)$', line))}))
            wait_for('repair still requires fresh PIN binding', lambda: old_key_denied(port))
            time.sleep(.1)
            sources[0].paired = False
            assert sources[0].start(port, repaired['pairing_pin']) == keys[0], 'repair changed receiver identity'
            restored, restored_d = wait_for('repaired source A resumes with source B', lambda: live([0, 1]), seconds=15)
            current = {session['receiver_id']: session for session in restored['sessions']}
            restored_a, continuous_b = current[receiver_ids[0]], current[receiver_ids[1]]
            assert restored_a['source_id'] == a_session['source_id'] and restored_a['session_id'] != a_session['session_id'], 'repair did not keep source identity with a fresh session'
            assert restored_a['playback_mode'] == 'synchronized' and restored_a['ingress']['latency_advance_ns'] == 0, 'source A did not use independent synchronized playback'
            assert continuous_b['playback_mode'] == 'low_latency' and continuous_b['ingress']['latency_advance_ns'] > 0, 'source B playback mode changed'
            wait_for('source B context preserved across repair', lambda: unchanged([1]))
            assert pids[0] in child_workers(hub), 'explicit repair restarted the existing idle worker'
            report['management_checks'].append(dict(operation='repair_same_key_with_new_pin', source_identity_preserved=True,
                fresh_session=True, source_B_context_unchanged=True, source_A_mode='synchronized', source_B_mode='low_latency',
                continuity=continuity_delta(after, after_d, restored, restored_d, [1])))
            report['scenarios']['explicit_repair_requires_new_pin_and_preserves_same_source_identity'] = True
            report['scenarios']['independent_playback_mode_does_not_reset_other_source'] = True
        elif args.scenario == 'mix-control':
            phase = 'mix-control baseline'
            initial = {row['receiver_id']: row for row in first['sessions']}
            original_b = initial[receiver_ids[1]]
            baseline_rms = [first_d['meters']['lanes'][initial[receiver_id]['lane']]['rms'] for receiver_id in receiver_ids]
            assert all(value > .001 for value in baseline_rms), 'both baseline source meters must be present and audible'
            report['mix_control_checks'] = []
            report['baseline_lane_rms'] = baseline_rms
            attenuation = 10 ** (-6 / 20)

            def mix_source(index, gain_db, muted=False, solo=False):
                current = next(row for row in airplay()['sessions'] if row['receiver_id'] == receiver_ids[index])
                command('mix_source', source_id=current['source_id'], session_id=current['session_id'],
                        gain_db=gain_db, muted=muted, solo=solo)

            def observe_ratios(label, desired):
                # Sample three separate 50 ms windows after the gain ramps settle.
                deadline = time.monotonic() + 5
                consecutive = 0
                while time.monotonic() < deadline:
                    snapshot, d = airplay(), diagnostics()
                    current = {row['receiver_id']: row for row in snapshot['sessions']}
                    b = current.get(receiver_ids[1])
                    assert b is not None, 'source B disappeared during targeted mix control'
                    for field in ['session_id', 'stream_id', 'stream_epoch', 'mapping_id']:
                        assert b[field] == original_b[field], 'targeted mix control changed source B context'
                    values = []
                    matches = True
                    for index, expected in desired.items():
                        row = current.get(receiver_ids[index])
                        if row is None or row['ingress']['accepted_packets'] < 20:
                            matches = False
                            break
                        lane = d['meters']['lanes'][row['lane']]
                        if lane['stream_id'] != row['stream_id']:
                            matches = False
                            break  # An absent/reassigned lane never counts as silence.
                        rms = lane['rms']
                        ratio = rms / baseline_rms[index]
                        values.append(dict(source_index=index + 1, rms=rms, ratio=ratio, expected_ratio=expected))
                        matches &= rms <= 1e-6 if expected == 0 else abs(ratio / expected - 1) <= .1
                    consecutive = consecutive + 1 if matches else 0
                    if consecutive >= 3:
                        report['mix_control_checks'].append(dict(operation=label, meters=values,
                            source_B_context_unchanged=True, stable_50ms_windows=3))
                        return snapshot, d
                    time.sleep(.05)
                raise AssertionError('mix meters failed to settle within 10 percent: ' + label)

            phase = 'targeted gain and mute'
            mix_source(0, -6)
            observe_ratios('A_minus_6dB_B_unchanged', {0: attenuation, 1: 1})
            mix_source(0, -6, muted=True)
            observe_ratios('A_muted_B_unchanged', {0: 0, 1: 1})
            phase = 'single and multiple Solo selection'
            mix_source(0, -6, solo=True)
            before_solo = next(row for row in airplay()['sessions'] if row['receiver_id'] == receiver_ids[1])['ingress']['accepted_packets']
            solo_snapshot, _ = observe_ratios('A_solo_B_silent', {0: attenuation, 1: 0})
            b = next(row for row in solo_snapshot['sessions'] if row['receiver_id'] == receiver_ids[1])
            assert b['ingress']['accepted_packets'] > before_solo, 'Solo stopped nonselected source ingress'
            mix_source(1, 0, solo=True)
            observe_ratios('A_and_B_multi_solo', {0: attenuation, 1: 1})
            mix_source(0, -6)
            mix_source(1, 0)
            observe_ratios('clear_all_solo', {0: attenuation, 1: 1})

            phase = 'persist gain and mute but clear Solo on reconnect'
            mix_source(0, -6, muted=True, solo=True)
            before_disconnect, before_d = observe_ratios('muted_A_solo_before_disconnect', {0: 0, 1: 0})
            old_a = next(row for row in before_disconnect['sessions'] if row['receiver_id'] == receiver_ids[0])
            command('disconnect_source', source_id=old_a['source_id'], session_id=old_a['session_id'])
            sources[0].close()
            wait_for('A disconnected for preference persistence', lambda: not any(row['receiver_id'] == receiver_ids[0] for row in airplay()['sessions']))
            disconnected, disconnected_d = observe_ratios('B_audible_after_solo_owner_disconnect', {1: 1})
            command('allow_source', source_id=old_a['source_id'])
            listing = subprocess.run(['/usr/sbin/lsof', '-a', '-p', str(pids[0]), '-iTCP', '-sTCP:LISTEN', '-Fn'], capture_output=True, text=True, check=True)
            ports = {int(match[1]) for line in listing.stdout.splitlines() if line.startswith('n') and (match := re.search(r':(\d+)$', line))}
            assert len(ports) == 1, 'receiver listener changed unexpectedly'
            port = ports.pop()
            def known_key_ready():
                attempt = DigitalSource(0)
                extra_sources.append(attempt)
                try:
                    attempt.control = socket.create_connection(('127.0.0.1', port), timeout=4)
                    attempt.sockets.append(attempt.control)
                    xpub = attempt.crypto.primitive('probe_x_public', os.urandom(32))
                    code, _ = attempt.rtsp('POST', '/pair-verify', bytes([1, 0, 0, 0]) + xpub + sources[0].public)
                    assert code in [200, 403], 'unexpected allow propagation response'
                    return code == 200
                finally:
                    attempt.close()
            wait_for('allow reaches verified worker trust', known_key_ready)
            time.sleep(.1)
            assert sources[0].start(port, receiver(receiver_ids[0]).get('pairing_pin', '')) == keys[0], 'reconnect changed receiver key'
            restored, restored_d = observe_ratios('A_reconnect_retains_mute_B_audible', {0: 0, 1: 1})
            a = next(row for row in restored['sessions'] if row['receiver_id'] == receiver_ids[0])
            assert a['source_id'] == old_a['source_id'] and a['session_id'] != old_a['session_id'], 'same source did not acquire a fresh session'
            assert a['mix']['gain_db'] == -6 and a['mix']['muted'] and not a['mix']['solo'], 'reconnect did not persist gain/mute and clear Solo'
            mix_source(0, -6, muted=False, solo=False)
            observe_ratios('explicit_unmute_restores_persisted_minus_6dB', {0: attenuation, 1: 1})
            report['mix_control_reconnect'] = dict(gain_db=-6, muted_preference_preserved=True, solo_cleared=True,
                source_identity_preserved=True, new_session=True, source_B_context_unchanged=True,
                continuity=continuity_delta(disconnected, disconnected_d, restored, restored_d, [1]))
            report['scenarios']['targeted_gain_mute_single_solo_multi_solo_and_clear'] = True
            report['scenarios']['gain_mute_persist_and_solo_clears_on_same_source_reconnect'] = True
        else:
            if args.sources:
                def occupied_hidden():
                    current = airplay()
                    return all(r['ready'] and r['active'] and r['discovery_state'] == 'hidden'
                               and r['visibility_reason'] == 'occupied' for r in current['receivers'][:args.sources])
                wait_for('occupied receivers locally hidden', occupied_hidden, seconds=10)
            phase = 'steady multi-source output'
            time.sleep(max(11, args.steady_seconds) if args.sources else args.steady_seconds)
            steady, steady_d = wait_for('steady multi-source output', lambda: live(range(args.sources)))
            for session in steady['sessions']:
                old = next(s for s in first['sessions'] if s['receiver_id'] == session['receiver_id'])
                assert session['released_blocks'] > old['released_blocks'], 'source PCM stopped during steady mixing'
            assert steady_d['output_frames'] > first_d['output_frames'], 'output callback stopped'
            if args.sources:
                report['scenarios']['distinct_receiver_and_source_identities_concurrent_encrypted_pcm'] = True
            assert len(set(native_streams)) == args.native_sources, 'native sources reused stream identity'
            if args.native_sources:
                report['scenarios']['independent_native_tones_render_with_requested_mix'] = True
            report['scenarios']['dynamic_unique_lanes_and_shared_output'] = True
            report['native_streams'] = len(native_streams)
            if args.sources:
                assert occupied_hidden(), 'occupied worker failed or republished after ten seconds'
                report['scenarios']['occupied_receivers_locally_hidden_beyond_ten_seconds'] = True
                report['visibility_scope'] = 'Hub discovery owner state only; Apple picker/cache visibility untested'
                phase = 'idempotent command and old session rejection'
                target = steady['sessions'][0]
                replay = dict(command_id=str(uuid.uuid4()), expected_revision=airplay()['revision'], operation=dict(
                    action='mix_source', source_id=target['source_id'], session_id=target['session_id'], gain_db=0, muted=False, solo=False))
                code, accepted = api('/v2/airplay', replay)
                assert code == 200, 'initial idempotency command rejected'
                code, repeated = api('/v2/airplay', replay)
                for response in [accepted, repeated]:
                    for row in response.get('receivers', []):
                        row.pop('pairing_pin', None)
                assert code == 200 and accepted == repeated, 'exact command replay changed result beyond PIN redaction'
                changed = dict(replay, operation=dict(replay['operation'], muted=True))
                code, refused = api('/v2/airplay', changed)
                assert code == 409 and refused.get('error') == 'command_id_reused', 'command id accepted a different payload'
                stale = dict(command_id=str(uuid.uuid4()), expected_revision=airplay()['revision'], operation=dict(
                    action='disconnect_source', source_id=target['source_id'], session_id=target['session_id'] + 1))
                code, refused = api('/v2/airplay', stale)
                assert code == 409 and refused.get('error') == 'session_changed', 'old session mutation was accepted'
                wait_for('sources survive invalid commands', lambda: live(range(args.sources)))
                report['scenarios']['command_replay_idempotent_and_stale_session_rejected'] = True
            if args.sources + args.native_sources == 4:
                phase = 'four inputs still permit another native Sender'
                extra = subprocess.run([str(DEV), str(binary), 'send', '--credential', str(state / f'native-{args.native_sources + 1}.json'),
                    '--hub', url, '--seconds', '1', '--frequency', '1511'], capture_output=True, cwd=ROOT, env=env, timeout=10)
                assert extra.returncode == 0, 'AirPlay quota incorrectly rejected a native Sender'
                wait_for('four sources continue after additional native Sender', lambda: live(range(args.sources)))
                assert airplay()['capacity']['active'] == args.sources and airplay()['capacity']['reserved'] == 0, 'native Sender changed AirPlay capacity'
                report['scenarios']['native_start_independent_of_airplay_quota'] = True
            report['steady'] = dict(output_rms=steady_d['meters']['output']['rms'], active=steady['capacity']['active'],
                                    lane_rms=[steady_d['meters']['lanes'][s['lane']]['rms'] for s in steady['sessions']],
                                    native_lane_rms=[next(lane['rms'] for lane in steady_d['meters']['lanes'] if lane['stream_id'] == stream_id) for stream_id in native_streams])

            report['steady_continuity'] = continuity_delta(first, first_d, steady, steady_d, range(args.sources))
            report['continuity_claim'] = 'Steady digital counters must remain continuous; this is not analog or Apple-device audio verification'
            phase = 'steady digital timing quality'
            assert_continuity(report['steady_continuity'], 'steady')
            report['scenarios']['steady_digital_timing_quality'] = True
            for cycle in range(args.fault_cycles):
                for operation in (['disconnect', 'worker_crash'] if args.sources + args.native_sources > 1 else []):
                    for victim in range(args.sources):
                        phase = f'cycle {cycle + 1} {operation} source {victim + 1}'
                        baseline, baseline_d = wait_for('all sources before baseline', lambda: live(range(args.sources)))
                        time.sleep(.3)
                        before, before_d = wait_for('all sources before isolation', lambda: live(range(args.sources)))
                        previous = {s['receiver_id']: s for s in before['sessions']}
                        target = previous[receiver_ids[victim]]
                        survivors = [i for i in range(args.sources) if i != victim]
                        if operation == 'disconnect':
                            command('disconnect_source', source_id=target['source_id'], session_id=target['session_id'])
                        else:
                            assert pids[victim] in child_workers(hub), 'victim is no longer a direct child of this fixture Hub'
                            os.kill(pids[victim], signal.SIGKILL)
                        sources[victim].close()

                        def survivors_continue():
                            value = live(survivors)
                            if not value:
                                return None
                            snapshot, d = value
                            if any(s['receiver_id'] == receiver_ids[victim] for s in snapshot['sessions']):
                                return None
                            if d['output_frames'] < before_d['output_frames'] + 4800:
                                return None
                            victim_meter = d['meters']['lanes'][target['lane']]
                            if victim_meter['rms'] is None:
                                # An inactive binding has no attributable PCM measurement.
                                # Confirm actual callback retirement, rather than inventing zero.
                                if (victim_meter.get('available') is not False
                                    or victim_meter['stream_id'] != 0
                                    or d['render_state_by_lane'][target['lane']] not in [0, 4]):
                                    return None
                            elif victim_meter['rms'] > 1e-6:
                                return None
                            for session in snapshot['sessions']:
                                old = previous[session['receiver_id']]
                                assert session['session_id'] == old['session_id'], 'one source failure replaced another session'
                                if session['released_blocks'] < old['released_blocks'] + 10:
                                    return None
                            return snapshot, d

                        after, after_d = wait_for('survivors continue and victim lane clears', survivors_continue, seconds=15)
                        report['isolation_checks'].append(dict(cycle=cycle + 1, operation=operation, source_index=victim + 1,
                            surviving_sources=len(after['sessions']) + args.native_sources, surviving_airplay_sources=len(after['sessions']),
                            surviving_native_sources=args.native_sources, output_frames_advanced=after_d['output_frames'] - before_d['output_frames'],
                            unchanged_survivor_sessions=True, victim_lane_cleared=True,
                            baseline=continuity_delta(baseline, baseline_d, before, before_d, survivors),
                            fault_interval=continuity_delta(before, before_d, after, after_d, survivors)))
                        if operation == 'disconnect':
                            command('allow_source', source_id=target['source_id'])
                            command('disable_receiver', receiver_id=receiver_ids[victim])
                        wait_for('victim worker exited', lambda: pids[victim] not in child_workers(hub))
                        # Hub clears its worker command handle after the child exits.
                        wait_for('victim receiver stopped', lambda: not receiver(receiver_ids[victim])['enabled'])
                        time.sleep(.2)
                        phase = f'cycle {cycle + 1} {operation} source {victim + 1} recovery startup'
                        ready, pids[victim], port = enable(receiver_ids[victim])
                        phase = f'cycle {cycle + 1} {operation} source {victim + 1} recovery authentication'
                        assert sources[victim].start(port, ready['pairing_pin']) == keys[victim], 'receiver key changed on recovery'
                        phase = f'cycle {cycle + 1} {operation} source {victim + 1} recovery PCM'
                        restored, restored_d = wait_for('paired source restored alongside survivors', lambda: live(range(args.sources)), seconds=15)
                        restored_session = next(s for s in restored['sessions'] if s['receiver_id'] == receiver_ids[victim])
                        assert restored_session['source_id'] == source_ids[receiver_ids[victim]], 'paired reconnect changed source identity'
                        assert restored_session['session_id'] != target['session_id'], 'reconnect reused the old session context'
                        report['isolation_checks'][-1]['recovery_interval'] = continuity_delta(after, after_d, restored, restored_d, survivors)
                        phase = f'cycle {cycle + 1} {operation} source {victim + 1} recovery digital timing quality'
                        for window in ('baseline', 'fault_interval', 'recovery_interval'):
                            assert_continuity(report['isolation_checks'][-1][window], window)
            if args.sources and args.sources + args.native_sources > 1:
                report['scenarios']['each_source_disconnect_preserves_other_sources'] = True
                report['scenarios']['each_worker_crash_preserves_other_sources'] = True
                report['scenarios']['paired_recovery_preserves_source_and_receiver_identity'] = True
        if args.shutdown_active:
            phase='managed shutdown with active encrypted sources'
            running_workers=set(child_workers(hub))
            runtime_keys=list(state.rglob('runtime-key-*'))
            assert len(running_workers)==args.sources and len(runtime_keys)==args.sources
            identity_hashes={str(path.relative_to(state)):hashlib.sha256(path.read_bytes()).hexdigest()
                for path in state.rglob('*.json') if '.credentials' in path.parts}
            assert identity_hashes, 'identity fixture missing'
            before=time.monotonic()
            hub.stdin.write(b'{"version":1,"type":"stop"}\n');hub.stdin.flush();hub.stdin.close()
            code=hub.wait(timeout=7)
            events=[json.loads(line) for line in log.read_text().splitlines() if line.startswith('{')]
            complete=[event for event in events if event.get('event')=='shutdown_complete']
            worker_stops=[event for event in events if event.get('event')=='airplay_worker_stopped'][-args.sources:]
            assert code==0 and complete and complete[-1]['forced'] is False
            assert not list(state.rglob('runtime-key-*'))
            assert len(worker_stops)==args.sources and all(not event['forced'] and event['exit_code']==0 and event['cleanup_complete'] for event in worker_stops)
            for pid in running_workers:
                assert subprocess.run(['/bin/kill','-0',str(pid)],capture_output=True).returncode!=0
            report['managed_shutdown']=dict(elapsed_ms=round((time.monotonic()-before)*1000),workers=args.sources,
                exit_code=code,forced=False,runtime_keys_before=len(runtime_keys),runtime_keys_after=0,processes_exited=True)
            for source in sources+extra_sources:source.close()
            report['scenarios']['managed_active_shutdown_releases_workers_and_runtime_keys']=True
            if args.scenario=='shutdown':
                phase='same profile and port reopen, owner EOF cleanup'
                hub=subprocess.Popen([str(DEV),str(binary),'serve','--config',str(state/'server.json'),
                    '--listen',url.removeprefix('https://'),'--managed-control-stdin'],stdin=subprocess.PIPE,
                    stdout=log_handle,stderr=log_handle,cwd=ROOT,env=env)
                wait_for('same port and profile reopen',lambda:len(airplay()['receivers'])==args.sources,seconds=20)
                for receiver_id in receiver_ids:enable(receiver_id)
                assert len(child_workers(hub))==args.sources
                eof_started=time.monotonic();hub.stdin.close()
                assert hub.wait(timeout=7)==0
                assert not list(state.rglob('runtime-key-*'))
                after_hashes={str(path.relative_to(state)):hashlib.sha256(path.read_bytes()).hexdigest()
                    for path in state.rglob('*.json') if '.credentials' in path.parts}
                assert identity_hashes==after_hashes, 'persisted receiver identity changed'
                report['reopen_and_owner_eof']=dict(same_profile_and_port=True,identity_unchanged=True,
                    elapsed_ms=round((time.monotonic()-eof_started)*1000),workers=args.sources,runtime_keys_after=0)

        else:
            phase = 'final global disable withdraws discovery and releases reservations'
            command('disable')
            for source in sources + extra_sources:
                source.close()
            def globally_disabled():
                snapshot = airplay()
                return snapshot if (all(not row['enabled'] and not row['ready'] and not row['active']
                    and row['discovery_state'] == 'hidden' for row in snapshot['receivers'])
                    and snapshot['capacity']['reserved'] == 0 and not child_workers(hub)) else None
            stopped = wait_for('global disable fully stops and hides receivers', globally_disabled, seconds=15)
            report['global_disable'] = dict(receivers=[{key: row[key] for key in ['enabled', 'ready', 'active', 'discovery_state']}
                for row in stopped['receivers']], reserved=stopped['capacity']['reserved'], owned_worker_count=0)
            report['scenarios']['global_disable_stops_workers_hides_discovery_and_releases_reservations'] = True
        report['passed'] = True
    except Exception as error:
        report['failure_phase'] = phase
        report['failure_type'] = type(error).__name__
        report['source_protocol'] = [dict(source_index=source.index + 1, stage=source.protocol_stage, last_response=source.last_response, thread_errors=source.errors) for source in sources]
        # Do not serialize arbitrary assertion values from protocol helpers.
        trace = error.__traceback__
        while trace and trace.tb_next:
            trace = trace.tb_next
        if isinstance(error, AssertionError) and trace and trace.tb_frame.f_code.co_name == 'wait_for':
            report['wait_timeout'] = trace.tb_frame.f_locals.get('label')
        if isinstance(error, AssertionError) and trace and Path(trace.tb_frame.f_code.co_filename).resolve() == Path(__file__).resolve():
            report['failure'] = str(error)
        if trace and Path(trace.tb_frame.f_code.co_filename).name == 'auth_probe.py':
            values = trace.tb_frame.f_locals
            report['protocol_failure'] = dict(stage=trace.tb_frame.f_code.co_name,
                status=values.get('code') if isinstance(values.get('code'), int) else None,
                response_bytes=len(values['body']) if isinstance(values.get('body'), bytes) else None)
            if trace.tb_frame.f_code.co_name == 'pair_verify':
                report['protocol_failure']['step'] = 2 if 'server' in values else 1

        if api:
            try:
                code, snapshot = api('/v2/airplay')
                report['failure_airplay_http_status'] = code
                if code == 200:
                    fields = ['enabled', 'ready', 'active', 'error', 'failure_stage', 'received_blocks',
                              'released_blocks', 'rejected_blocks', 'queued_blocks', 'ingress', 'admission_denials', 'max_loop_gap_ns', 'max_control_ns', 'max_pcm_ns']
                    report['failure_receivers'] = [{key: receiver.get(key) for key in fields}
                                                   for receiver in snapshot['receivers']]
                    report['failure_capacity'] = snapshot['capacity']
                code, d = api('/v1/diagnostics')
                if code == 200:
                    report['failure_output'] = dict(output_frames=d['output_frames'],
                        output_stats=d['output_stats'],
                        timed_late_frames=d['timed_late_frames'],
                        timed_late_frames_by_lane=d['timed_late_frames_by_lane'],
                        last_timed_late={field: d[field] for field in ('last_timed_late_stream_id', 'last_timed_late_epoch', 'last_timed_late_output_frame', 'last_timed_late_target_ns', 'last_timed_late_presentation_ns')},
                        lane_rms=[lane['rms'] for lane in d['meters']['lanes']])
            except Exception:
                pass
    finally:
        readers_stop.set()
        for thread in readers:
            thread.join(timeout=5)
        report['status_reads'] = dict(readers=args.status_readers, successful=sum(reader_counts), errors=sum(reader_errors))
        for source in sources + extra_sources:
            source.close()
        report['source_pacing'] = [dict(source_index=source.index + 1, sent_packets=source.sent_packets,
            max_schedule_lag_ns=source.max_schedule_lag_ns, max_send_gap_ns=source.max_send_gap_ns)
            for source in sources]
        for process in native_processes:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        for handle in native_logs:
            handle.close()
        if api and not report.get('global_disable'):
            try:
                code, snapshot = api('/v2/airplay')
                if code == 200:
                    api('/v2/airplay', dict(command_id=str(uuid.uuid4()), expected_revision=snapshot['revision'], operation=dict(action='disable')))
            except Exception:
                pass
        if hub and hub.poll() is None:
            hub.terminate()
            try:
                hub.wait(timeout=10)
            except subprocess.TimeoutExpired:
                hub.kill()
                hub.wait(timeout=5)
        if log_handle:
            log_handle.close()
        report['fixture_process_stopped'] = hub is None or hub.poll() is not None
        report['native_processes_stopped'] = all(process.poll() is not None for process in native_processes)
        assert base.parent == ROOT / '.local/airplay-multi' and base.name == run and not base.is_symlink(), 'refusing unrelated fixture cleanup'
        shutil.rmtree(base)
        report['fixture_files_removed'] = not base.exists()
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    summary = {key: report[key] for key in ['passed', 'scenario', 'sources', 'native_sources', 'fault_cycles', 'binary_sha256', 'failure_phase', 'failure_type', 'failure', 'protocol_failure'] if key in report}
    summary.update(report=str(report_path.relative_to(ROOT)), fault_observations=len(report['isolation_checks']),
                   recoveries_completed=sum('recovery_interval' in check for check in report['isolation_checks']),
                   management_checks_completed=len(report.get('management_checks', [])),
                   mix_control_checks_completed=len(report.get('mix_control_checks', [])))
    print(json.dumps(summary, ensure_ascii=False))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
