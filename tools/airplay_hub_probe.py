#!/usr/bin/env python3
"""macOS fixture: real Hub TLS API, Speaker worker lifetime and native SRTP audio.

All fixtures are local to this project. This uses explicit `init` laboratory
credentials; it does not prove production pairing-token storage or iPhone/macOS
AirPlay playback. The selected CoreAudio device is real, at the default -12 dB
room gain with a -36 dB synthetic native Sender.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import ssl
import subprocess
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen
import uuid

from credential_fixture import receiver_snapshot, remove_owned_fixture

ROOT = Path(__file__).resolve().parents[1]
DEV = ROOT / 'tools/dev'


def wait_for(label, check, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (OSError, ValueError, URLError):
            pass
        time.sleep(0.1)
    raise AssertionError(f'{label}: timeout')



def airplay_session(snapshot, source_id=None):
    """Find this probe's active v2 session; never infer a physical Mixer lane."""
    sessions = [session for session in snapshot['sessions']
                if source_id is None or session['source_id'] == source_id]
    if len(sessions) != 1:
        return None
    session = sessions[0]
    assert isinstance(session['lane'], int) and session['lane'] >= 0, 'invalid active AirPlay lane'
    assert session['stream_id'] > 0 and session['session_id'] > 0, 'invalid active AirPlay context'
    return session


def session_meter(diagnostics, session):
    """Reject stale per-lane meters until they carry the selected stream ID."""
    if session is None:
        return None
    lanes = diagnostics['meters']['lanes']
    assert session['lane'] < len(lanes), 'AirPlay lane is outside Mixer diagnostics'
    meter = lanes[session['lane']]
    return meter if meter['stream_id'] == session['stream_id'] else None


def child_workers(hub):
    """Only exact-name direct children of this still-running fixture Hub."""
    if hub.poll() is not None:
        return []
    result = subprocess.run(['/bin/ps', '-axo', 'pid=,ppid=,comm='],
                            capture_output=True, text=True, check=True)
    workers = []
    for line in result.stdout.splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) == 3 and int(fields[1]) == hub.pid:
            if Path(fields[2]).name == 'neonmix-airplay-worker':
                workers.append(int(fields[0]))
    return workers


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', choices=['debug', 'release'], default='debug')
    parser.add_argument('--output', default='coreaudio:BuiltInSpeakerDevice')
    parser.add_argument('--report', type=Path, help='Project-local result file; defaults to a new artifacts/credential-storage run')
    args = parser.parse_args()
    report_path = (args.report or ROOT / 'artifacts/credential-storage' / ('airplay-hub-' + time.strftime('%Y%m%d-%H%M%S') + '-' + uuid.uuid4().hex[:8]) / 'result.json').resolve()
    report_path.relative_to(ROOT)
    assert platform.system() == 'Darwin', 'macOS-only probe'
    hub_binary = ROOT / 'target' / args.profile / 'neonmix-hub'
    worker_binary = hub_binary.with_name('neonmix-airplay-worker')
    assert hub_binary.is_file() and worker_binary.is_file(), 'build Hub and worker first'
    assert (ROOT / '.local/airplay/plugins').is_dir(), 'prepare the audio-only plugin runtime first'
    base = ROOT / '.local/tmp' / f'airplay-hub-{uuid.uuid4().hex[:12]}'
    base.mkdir(mode=0o700, parents=True)
    state = base / 'profile'
    state.mkdir(mode=0o700)
    processes = []
    handles = []
    hub = None
    credentials = {}
    request_api = None
    report = {
        'platform': 'macOS',
        'build_profile': args.profile,
        'scope': 'isolated loopback TLS Hub API, real CoreAudio output, integrated Speaker worker lifecycle, synthetic native SRTP Sender',
        'credential_scope': 'explicit init laboratory bearer tokens; not production credential-storage acceptance',
        'binary_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in [hub_binary, worker_binary]},
        'output_device': args.output,
        'room_gain_db': -12,
        'native_sender_gain_db': -36,
        'passed': False,
        'scenarios': {},
        'limitations': ['no Apple AirPlay sender was attached', 'no analog latency or audio/video synchronization measurement', 'no output unplug/loss injection'],
    }
    def spawn(*arguments):
        log_path = base / f'process-{len(processes)}.log'
        handle = log_path.open('w')
        handles.append(handle)
        env = os.environ.copy()
        env['NEONMIX_AIRPLAY_RUNTIME'] = str(ROOT / '.local/airplay')
        process = subprocess.Popen([str(DEV), str(hub_binary), *map(str, arguments)],
                                   stdout=handle, stderr=handle, cwd=ROOT, env=env)
        processes.append(process)
        return process, log_path
    def cli(*arguments):
        result = subprocess.run([str(DEV), str(hub_binary), *map(str, arguments)],
                                capture_output=True, cwd=ROOT, timeout=30)
        assert result.returncode == 0, 'fixture Hub init failed'
    def redact_failure(error):
        # Never copy raw server responses, process logs, tokens, paths or identities.
        if isinstance(error, AssertionError):
            return str(error)
        return type(error).__name__
    try:
        cli('init', '--directory', state, '--output', args.output)
        for name in ['admin', 'sender-a']:
            credentials[name] = json.loads((state / f'{name}.json').read_text())
        tls = ssl.create_default_context(cadata=credentials['admin']['certificate'])
        tls.minimum_version = ssl.TLSVersion.TLSv1_3
        tls.maximum_version = ssl.TLSVersion.TLSv1_3
        hub, hub_log = spawn('serve', '--config', state / 'server.json', '--listen', '127.0.0.1:0')
        def started():
            assert hub.poll() is None, 'fixture Hub exited before startup'
            for line in hub_log.read_text().splitlines():
                try:
                    event = json.loads(line)
                except ValueError:
                    continue
                if event.get('event') == 'hub_started':
                    return event['listen']
            return None
        listen = wait_for('Hub listening', started)
        url = f'https://{listen}'
        def api(path, role='admin', body=None):
            payload = None if body is None else json.dumps(body).encode()
            request = Request(url + path, data=payload,
                              headers={'Authorization': 'Bearer ' + credentials[role]['token'],
                                       'Content-Type': 'application/json'})
            try:
                with urlopen(request, context=tls, timeout=4) as response:
                    return response.status, json.load(response)
            except HTTPError as error:
                return error.code, json.load(error)
        request_api = api
        def get(path, role='admin'):
            status, body = api(path, role)
            assert status == 200, 'fixture authenticated GET failed'
            return body
        def speaker():
            return get('/v1/airplay')
        def diagnostics():
            return get('/v1/diagnostics')
        def command(action, role='admin', revision=None):
            if revision is None:
                revision = speaker()['revision']
            return api('/v1/airplay', role, {'expected_revision': revision, 'operation': {'action': action}})
        baseline = wait_for('CoreAudio output running', lambda: (d if d['output_frames'] > 4800 else None) if (d := diagnostics()) else None)
        snapshot = get('/v1/hub')
        assert snapshot['output']['available'], 'real output unavailable'
        assert snapshot['output']['gain_db'] == -12, 'room gain was not default -12 dB'
        report['scenarios']['tls13_trusted_fixture_certificate_and_real_output'] = True
        report['output_frames_at_start'] = baseline['output_frames']
        identity = None
        for round_index in range(2):
            status, _ = command('enable')
            assert status == 200, 'administrator enable rejected'
            ready = wait_for('Speaker worker ready', lambda: (s if s['enabled'] and s['ready'] else None) if (s := speaker()) else None, seconds=25)
            current_identity = receiver_snapshot(state / 'airplay/receiver.json')
            if identity is None:
                identity = current_identity
            else:
                assert current_identity == identity, 'receiver identity or trust changed on restart'
            assert ready.get('pairing_pin') and len(ready['pairing_pin']) == 4, 'admin pairing PIN missing'
            assert 'pairing_pin' not in get('/v1/airplay', 'sender-a'), 'member response exposed pairing PIN'
            assert 'pairing_pin' not in json.dumps(diagnostics()['airplay']), 'diagnostics exposed pairing PIN'
            assert get('/v1/me', 'sender-a')['role'] == 'member', 'fixture member role missing'
            status, body = command('disable', 'sender-a')
            assert status == 403 and body.get('error') == 'permission_denied', 'member AirPlay write was not forbidden'
            revision = ready['revision']
            status, body = command('allow', revision=revision - 1)
            assert status == 409 and body.get('error') == 'revision_conflict', 'stale AirPlay revision was accepted'
            status, _ = command('disable')
            assert status == 200, 'administrator disable rejected'
            wait_for('worker stopped after disable', lambda: not child_workers(hub))
            wait_for('Speaker disabled', lambda: not speaker()['enabled'] and not speaker()['ready'])
            time.sleep(0.2)
        report['scenarios']['two_enable_disable_rounds_and_worker_cleanup'] = True
        report['scenarios']['receiver_file_identity_and_trust_survive_worker_restart'] = True
        report['scenarios']['admin_only_pin_member_write_denied_stale_revision_denied'] = True
        receiver_path = state / 'airplay/receiver.json'
        receiver_bytes = receiver_path.read_bytes()
        key_path = receiver_path.parent / '.credentials' / (identity[0]['key_reference'] + '.json')
        key_bytes = key_path.read_bytes()
        for failure in ['missing', 'corrupt']:
            try:
                if failure == 'missing':
                    key_path.unlink()
                else:
                    key_path.write_text('{', encoding='utf-8')
                status, _ = command('enable')
                assert status == 200, 'credential failure fixture enable rejected'
                failed = wait_for('bad receiver key rejected', lambda: (s if not s['enabled'] and s.get('error') == 'worker_failed' else None) if (s := speaker()) else None)
                assert not failed['ready'] and not child_workers(hub), 'bad receiver identity started worker'
                assert receiver_path.read_bytes() == receiver_bytes, 'bad key regenerated receiver identity'
                assert not list(receiver_path.parent.glob('runtime-key-*')), 'failed startup retained runtime PEM'
            finally:
                key_path.write_bytes(key_bytes)
                key_path.chmod(0o600)
        assert receiver_snapshot(receiver_path) == identity, 'failed startup changed receiver trust/key'
        report['scenarios']['missing_corrupt_receiver_key_fails_without_identity_replacement'] = True
        sender, _ = spawn('send', '--credential', state / 'sender-a.json', '--hub', url, '--seconds', '60', '--frequency', '437')
        def native_ready():
            assert sender.poll() is None, 'native Sender exited'
            streams = get('/v1/hub')['streams']
            if len(streams) != 1:
                return None
            stream_id = next(iter(streams.values()))['id']
            d = diagnostics()
            lane = next((item for item in d['meters']['lanes'] if item['stream_id'] == stream_id), None)
            return stream_id if lane and lane['rms'] > 0.0005 else None
        native_id = wait_for('authenticated native Sender audible meter', native_ready, seconds=20)
        status, _ = command('enable')
        assert status == 200, 'crash fixture enable rejected'
        wait_for('crash fixture worker ready', lambda: speaker()['ready'], seconds=25)
        workers = wait_for('fixture child worker PID', lambda: child_workers(hub))
        assert len(workers) == 1, 'unexpected fixture worker child count'
        victim = workers[0]
        before = diagnostics()
        # Revalidate ownership directly before killing; no global pkill/name match.
        assert victim in child_workers(hub), 'worker ownership changed before kill'
        os.kill(victim, signal.SIGKILL)
        failed = wait_for('worker crash detected', lambda: (s if not s['enabled'] and s.get('error') == 'worker_failed' else None) if (s := speaker()) else None)
        assert not failed['active'] and 'pairing_pin' not in failed, 'failed worker left session/PIN active'
        assert hub.poll() is None and sender.poll() is None, 'worker crash terminated Hub or native Sender'
        def continued():
            d = diagnostics()
            lane = next((item for item in d['meters']['lanes'] if item['stream_id'] == native_id), None)
            return d if d['output_frames'] >= before['output_frames'] + 4800 and lane and lane['rms'] > 0.0005 else None
        after = wait_for('native PCM continues after worker crash', continued)
        assert len(get('/v1/hub')['streams']) == 1, 'native session lost after worker crash'
        report['scenarios']['sigkill_worker_preserves_hub_output_and_authenticated_native_sender'] = True
        report['output_frames_around_worker_crash'] = {'before': before['output_frames'], 'after': after['output_frames']}
        report['native_lane_rms_after_worker_crash'] = next(item['rms'] for item in after['meters']['lanes'] if item['stream_id'] == native_id)
        report['passed'] = True
    except Exception as error:
        report['failure'] = redact_failure(error)
    finally:
        if request_api:
            try:
                _, state_body = request_api('/v1/airplay')
                request_api('/v1/airplay', body={'expected_revision': state_body['revision'], 'operation': {'action': 'disable'}})
            except (OSError, ValueError, URLError):
                pass
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        for handle in handles:
            handle.close()
        report['fixture_processes_stopped']=all(process.poll() is not None for process in processes)
        remove_owned_fixture(base)
        report['fixture_files_removed']=not base.exists()
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(report, ensure_ascii=False))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
