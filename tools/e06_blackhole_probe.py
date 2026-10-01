#!/usr/bin/env python3
"""macOS E06: ordinary afplay -> default BlackHole output -> native capture / SRTP Sender.
Temporarily selects BlackHole and changes its volume, mute and rate; restores all settings.
Generated WAV, credentials, logs and results remain inside the project.
"""
import argparse
import hashlib
import http.client
import json
import math
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
import wave

from macos_audio_controls import DeviceControls

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', required=True, help='Explicit Hub output, different from BlackHole')
parser.add_argument('--provider', choices=['blackhole','neonmix'], default='blackhole')
parser.add_argument('--stop-signal', choices=['SIGINT','SIGTERM'], default='SIGINT')
args = parser.parse_args()
DEVICE = 'coreaudio:' + ('BlackHole2ch_UID' if args.provider == 'blackhole' else 'com.neonmix.audio.virtual-output')
if sys.platform != 'darwin' or args.output == DEVICE:
    parser.error('macOS and a Hub output different from BlackHole are required')
audio, hub_bin = ROOT / 'target/release/neonmix-audio', ROOT / 'target/release/neonmix-hub'
out = ROOT / ('artifacts/e06-blackhole' if args.provider == 'blackhole' else 'artifacts/e06-macos-hal') / time.strftime('%Y%m%d-%H%M%S')
out.mkdir(parents=True)
lab = Path(tempfile.mkdtemp(prefix='e06-blackhole-', dir=ROOT / '.local/tmp'))
controls = DeviceControls(DEVICE.removeprefix('coreaudio:'))
original = {'rate': controls.rate(), 'volume': controls.volume(), 'mute': controls.muted(), 'defaults': controls.default_outputs()}
children, handles = [], []
settings_changed = False
report = {'passed': False, 'device': DEVICE, 'output': args.output, 'original_settings': original,
          'stop_signal': args.stop_signal,
          'provider': args.provider,
          'scope': ('BlackHole external-driver path; not NeonMix HAL system-load acceptance' if args.provider == 'blackhole' else 'installed NeonMix HAL: system selection, afplay, native capture and encrypted Sender'),
          'binary_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in (audio, hub_bin)}}


def start(name, argv):
    stdout, stderr = (out / f'{name}.jsonl').open('w'), (out / f'{name}.stderr').open('w')
    handles.extend((stdout, stderr))
    child = subprocess.Popen([str(arg) for arg in argv], cwd=ROOT, stdout=stdout, stderr=stderr)
    children.append(child)
    return child


def rows(name):
    values = []
    for line in (out / f'{name}.jsonl').read_text().splitlines():
        try:
            values.append(json.loads(line))
        except ValueError:
            pass
    return values


def wait_for(getter, predicate=lambda value: bool(value), seconds=10):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = getter()
        if predicate(value):
            return value
        time.sleep(.1)
    raise TimeoutError('expected state did not arrive')


def stop(child):
    if child.poll() is None:
        child.send_signal(signal.SIGINT)
        try:
            child.wait(timeout=8)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()


def capture_stat():
    values = [row for row in rows('capture') if row['event'] == 'capture_stats']
    return values[-1] if values else None


def measure_phase(name):
    # Take both ends of the measurement after the setting has settled, from separate ticks.
    current = capture_stat()
    before = wait_for(capture_stat, lambda r: r and (not current or r['stats']['frames'] > current['stats']['frames']))
    time.sleep(2.05)
    after = wait_for(capture_stat, lambda r: r and r['stats']['frames'] > before['stats']['frames'])
    a, b = before['measurement'], after['measurement']
    frames = b['analyzed_frames'] - a['analyzed_frames']
    rms = math.sqrt(max(0, (b['rms']**2 * b['analyzed_frames'] - a['rms']**2 * a['analyzed_frames']) / frames))
    silent = (after['stats']['silent_frames'] - before['stats']['silent_frames']) / (after['stats']['frames'] - before['stats']['frames'])
    assert frames >= 48000
    assert after['stats']['last_device_ns'] > before['stats']['last_device_ns']
    assert not after['no_data']
    assert all(after['stats'][key] == 0 for key in ('errors', 'callback_over_budget', 'dropped_frames', 'stale_frames'))
    value = {'rms': rms, 'silent_fraction': silent, 'frames': frames, 'before': before, 'after': after}
    report.setdefault('capture_phases', {})[name] = value
    print(json.dumps({'phase': name, 'rms': rms, 'silent_fraction': silent}), flush=True)
    return value


def api(path, method='GET', body=None):
    connection = http.client.HTTPSConnection('localhost', port, context=context, timeout=5)
    try:
        connection.request(method, path, body=None if body is None else json.dumps(body),
                           headers={'Authorization': 'Bearer ' + credential['token'], 'Content-Type': 'application/json'})
        response = connection.getresponse()
        value = json.loads(response.read())
        assert response.status == 200, value
        return value
    finally:
        connection.close()


def diagnostic():
    try:
        return api('/v1/diagnostics')
    except (OSError, AssertionError):
        return None


def sender(name, seconds=25):
    return start(name, [hub_bin, 'send', '--credential', lab / 'sender-a.json', '--hub', f'https://localhost:{port}',
                        '--virtual-output', '--virtual-output-provider', args.provider, '--seconds', seconds])


def active_session():
    return next((s for s in api('/v1/hub')['sessions'].values() if s['status'] in ('buffering', 'playing', 'network_degraded')), None)


try:
    # Existing tests may own this global device. Report the conflict before changing it.
    busy = []
    for line in subprocess.check_output(['ps', '-ww', '-axo', 'pid=,command='], text=True).splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) != 2:
            continue
        argv = shlex.split(fields[1])
        if not argv or Path(argv[0]).name not in ('neonmix-hub', 'neonmix-audio'):
            continue
        if len(argv) > 1 and argv[1] in ('play', 'capture') and '--device' in argv and argv[argv.index('--device') + 1] == DEVICE:
            busy.append(int(fields[0]))
        if len(argv) > 1 and argv[1] == 'serve' and '--config' in argv:
            config = Path(argv[argv.index('--config') + 1])
            if config.exists() and json.loads(config.read_text()).get('output') == DEVICE:
                busy.append(int(fields[0]))
    if busy:
        report['blocked_by_device_owners'] = busy
        raise RuntimeError(f'{DEVICE} is in use by other NeonMix probes: {busy}')
    binding = subprocess.run([str(hub_bin), 'virtual-output', '--provider', args.provider], capture_output=True, text=True, check=True)
    report['binding'] = json.loads(binding.stdout)
    assert report['binding']['external_driver'] == (args.provider == 'blackhole') and report['binding']['device']['id'] == DEVICE
    settings_changed = True
    controls.set_rate(48000)
    controls.set_volume(1)
    controls.set_muted(False)
    controls.select_default_output()
    assert controls.default_outputs()['dOut'] == controls.device
    # This ordinary macOS player writes a 44.1kHz PCM16 file through the system-selected output.
    tone, second_tone = lab / 'tone.wav', lab / 'second-tone.wav'
    for path, frequency in [(tone, 437), (second_tone, 659)]:
        with wave.open(str(path), 'wb') as wav:
            wav.setnchannels(2)
            wav.setsampwidth(2)
            wav.setframerate(44100)
            second = b''.join(struct.pack('<hh', *([round(32767 * 10**(-36/20) * math.sin(2*math.pi*frequency*i/44100))]*2)) for i in range(44100))
            for _ in range(40):
                wav.writeframesraw(second)
    capture = start('capture', [audio, 'capture', '--device', DEVICE, '--rate', 48000, '--period', 480, '--seconds', 32])
    assert measure_phase('no_source')['silent_fraction'] > .99
    source = start('afplay', ['/usr/bin/afplay', tone])
    time.sleep(.5)
    baseline = measure_phase('full_volume')['rms']
    assert .010 < baseline < .0125
    frequency = report['capture_phases']['full_volume']['after']['measurement']['estimated_frequency_hz']
    assert frequency and abs(frequency - 437) < .5
    second_source = start('afplay-second', ['/usr/bin/afplay', second_tone])
    time.sleep(.3)
    mixed = measure_phase('two_applications')['rms']
    assert 1.37 < mixed / baseline < 1.46, 'ordinary application streams were not mixed correctly'
    stop(second_source)
    time.sleep(.3)
    controls.set_volume(.5)
    attenuation_db = controls.volume_db()
    expected_ratio = 10 ** (attenuation_db / 20)
    time.sleep(.3)
    half = measure_phase('half_volume')['rms']
    report['device_volume_transfer'] = {'scalar': .5, 'reported_db': attenuation_db,
                                        'expected_ratio': expected_ratio, 'measured_ratio': half / baseline}
    assert .95 < half / baseline / expected_ratio < 1.05, 'native volume was not applied exactly once'
    controls.set_muted(True)
    time.sleep(.3)
    muted = measure_phase('device_mute')
    assert muted['rms'] < .000001 and muted['silent_fraction'] > .99
    controls.set_volume(1)
    controls.set_muted(False)
    time.sleep(.3)
    restored = measure_phase('unmuted')['rms']
    assert .95 < restored / baseline < 1.05
    stop(source)
    time.sleep(.3)
    assert measure_phase('source_stopped')['silent_fraction'] > .99
    assert capture.wait(timeout=15) == 0
    report['capture_complete'] = next(row for row in rows('capture') if row['event'] == 'capture_complete')

    # Establish the same external device as an authenticated Sender input.
    subprocess.run([str(hub_bin), 'init', '--directory', str(lab), '--output', args.output], check=True, stdout=subprocess.DEVNULL)
    credential = json.loads((lab / 'admin.json').read_text())
    context = ssl.create_default_context(cadata=credential['certificate'])
    with socket.socket() as free:
        free.bind(('127.0.0.1', 0))
        port = free.getsockname()[1]
    hub = start('hub', [hub_bin, 'serve', '--config', lab / 'server.json', '--listen', f'127.0.0.1:{port}'])
    wait_for(diagnostic)
    source = start('afplay-sender', ['/usr/bin/afplay', tone])
    sending = sender('sender')
    steady = wait_for(diagnostic, lambda d: d and len(d['receivers']) == 1 and d['receivers'][0]['pcm_frames'] > 120000)
    report['sender_steady'] = steady
    receive = steady['receivers'][0]
    assert receive['authenticated'] and receive['last_buffer_rms'] > .008, f'bad receiver signal: {receive}'
    assert receive['packets']['timestamp_step_errors'] == receive['queue_drops'] == 0, f'bad receiver timeline: {receive}'
    first = active_session()
    assert first
    sending.send_signal(getattr(signal,args.stop_signal))
    assert sending.wait(timeout=8) == 0
    assert any(r['event'] == 'sender_stop_requested' for r in rows('sender'))
    assert any(r['event'] == 'capture_closed' for r in rows('sender'))
    stopped = wait_for(lambda: api('/v1/hub'), lambda s: s['sessions'][first['id']]['status'] == 'user_stopped')
    time.sleep(2)
    assert not active_session(), 'explicit stop automatically resumed'
    report['user_stopped'] = stopped['sessions'][first['id']]
    # Driver and ordinary source continue when Sender is gone.
    offline = subprocess.run([str(audio), 'capture', '--device', DEVICE, '--seconds', '3'], capture_output=True, text=True, timeout=10)
    assert offline.returncode == 0, offline.stderr
    final = next(json.loads(line) for line in offline.stdout.splitlines() if json.loads(line)['event'] == 'capture_complete')
    assert final['stats']['frames'] > 48000 and final['measurement']['rms'] > .008
    report['sender_offline_capture'] = final

    if args.provider == 'blackhole':
        # A native format invalidation must stop the old context. Restart is an explicit action.
        faulting = sender('sender-fault')
        old = wait_for(active_session)
        wait_for(diagnostic, lambda d: d and len(d['receivers']) == 1 and d['receivers'][0]['pcm_frames'] > 48000)
        controls.set_rate(44100)
        assert faulting.wait(timeout=10) != 0
        assert any(r['event'] == 'capture_fault' for r in rows('sender-fault'))
        report['native_fault'] = rows('sender-fault')
        assert not active_session()
        stop(source)
        controls.set_rate(44100)
        source = start('afplay-restart', ['/usr/bin/afplay', tone])
        restarted = sender('sender-restarted', 6)
        fresh = wait_for(active_session)
        assert fresh['id'] != old['id'] and fresh['media_context'] != old['media_context']
        restored = wait_for(diagnostic, lambda d: d and len(d['receivers']) == 1 and d['receivers'][0]['pcm_frames'] > 48000)
        assert restored['receivers'][0]['authenticated'] and restored['receivers'][0]['last_buffer_rms'] > .008
        assert restarted.wait(timeout=10) == 0
        opened = next(r for r in rows('sender-restarted') if r['event'] == 'sender_started')
        assert opened['capture']['format']['sample_rate'] == 44100
        report['fresh_session'] = {'session': fresh, 'sender_started': opened, 'diagnostic': restored}
    else:
        # The owned device is fixed-rate: rejecting a native rate change must
        # preserve its format and active audio instead of fabricating 44.1k support.
        rejected = False
        try: controls.set_rate(44100)
        except RuntimeError: rejected = True
        assert rejected and controls.rate() == 48000
        restarted = sender('sender-restarted', 6)
        fresh = wait_for(active_session)
        assert fresh['id'] != first['id'] and fresh['media_context'] != first['media_context']
        restored = wait_for(diagnostic, lambda d: d and len(d['receivers']) == 1 and d['receivers'][0]['pcm_frames'] > 48000)
        assert restored['receivers'][0]['authenticated'] and restored['receivers'][0]['last_buffer_rms'] > .008
        assert restarted.wait(timeout=10) == 0
        report['fixed_rate_rejected_without_change'] = True
        report['fresh_session'] = {'session': fresh, 'diagnostic': restored}
    report['passed'] = True
except Exception as error:
    report['error'] = repr(error)
    report['traceback'] = traceback.format_exc()
finally:
    for child in reversed(children):
        stop(child)
    for handle in handles:
        handle.close()
    restore_errors = []
    for name, action in ([('mute', lambda: controls.set_muted(original['mute'])),
                         ('volume', lambda: controls.set_volume(original['volume'])),
                         ('rate', lambda: controls.set_rate(original['rate'])),
                         ('default_outputs', lambda: controls.restore_default_outputs(original['defaults']))] if settings_changed else []):
        try:
            action()
        except Exception as error:
            restore_errors.append({'setting': name, 'error': repr(error)})
    report['restore_errors'] = restore_errors
    report['passed'] &= not restore_errors
    shutil.rmtree(lab)
    (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'error': report.get('error'), 'restore_errors': restore_errors, 'evidence': str(out)}, indent=2), flush=True)
raise SystemExit(0 if report['passed'] else 1)
