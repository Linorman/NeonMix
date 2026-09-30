#!/usr/bin/env python3
"""Real macOS dual Sender/Hub/native output soak with digital continuity checks."""
import argparse
import hashlib
import http.client
import json
from pathlib import Path
import fcntl
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import time
from macos_audio_controls import DeviceControls
from macos_process_stats import ProcessStats

ROOT = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--device', required=True)
p.add_argument('--seconds', type=int, default=1800)
args = p.parse_args()
if sys.platform != 'darwin' or not 60 <= args.seconds <= 30000:
    p.error('macOS, 60..30000 seconds')
# Serialize this probe before opening or changing the selected device.
lock_dir = ROOT / '.local/locks'
lock_dir.mkdir(parents=True, exist_ok=True)
probe_lock = (lock_dir / 'hub-soak.lock').open('a+')
try:
    fcntl.flock(probe_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
except BlockingIOError:
    p.error('another hub_soak_probe owns the device test')
hub_source, audio_source = ROOT / 'target/release/neonmix-hub', ROOT / 'target/release/neonmix-audio'
out = ROOT / 'artifacts/hub-soak' / time.strftime('%Y%m%d-%H%M%S')
out.mkdir(parents=True)
# Run immutable copies: another build cannot change later child launches.
(out / 'bin').mkdir()
hub_bin, audio = out / 'bin/neonmix-hub', out / 'bin/neonmix-audio'
shutil.copy2(hub_source, hub_bin)
shutil.copy2(audio_source, audio)
lab = Path(tempfile.mkdtemp(prefix='hub-soak-', dir=ROOT / '.local/tmp'))
controls = None
original_rate = None
children, handles = [], []
report = {'passed': False, 'seconds': args.seconds, 'device': args.device, 'hub_sha256': hashlib.sha256(hub_bin.read_bytes()).hexdigest(), 'audio_sha256': hashlib.sha256(audio.read_bytes()).hexdigest(), 'scope': 'real dual UDP/Opus/SRTP/native digital path on one macOS host; not cross-host analog latency'}
process_stats = ProcessStats()
named_processes = {}


def start(name, argv):
    log, err = (out / (name + '.jsonl')).open('w'), (out / (name + '.stderr')).open('w')
    handles.extend([log, err])
    child = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=err)
    children.append(child)
    if name != 'capture':  # Capture can finish between the final poll and sample.
        named_processes[name] = child.pid
    return child


def get(path):
    c = http.client.HTTPSConnection('localhost', port, context=context, timeout=5)
    try:
        c.request('GET', path, headers={'Authorization': 'Bearer ' + credential['token']})
        response = c.getresponse()
        assert response.status == 200
        return json.loads(response.read())
    finally:
        c.close()


def read_rows(name):
    values = []
    for line in (out / (name + '.jsonl')).read_text().splitlines():
        try:
            values.append(json.loads(line))
        except ValueError:
            pass
    return values


def validate_diagnostic(diagnostic):
    assert len(diagnostic['receivers']) == 2, 'receiver count changed'
    assert diagnostic['output_stats']['errors'] == 0, 'native output error'
    assert diagnostic['output_stats']['callback_over_budget'] == 0, 'native output callback exceeded budget'
    assert diagnostic['underrun_frames'] == 0, 'mixer underrun'
    assert all(r['queue_drops'] == 0 for r in diagnostic['receivers']), 'PCM queue overflow'
    assert all(r['pcm_timing_gap_count'] == 0 for r in diagnostic['receivers']), 'PCM source timeline gap'
    assert all(r['pcm_sink_dropped'] == 0 for r in diagnostic['receivers']), 'PCM appsink overflow'
    assert all(r['lost_packets'] == r['late_packets'] == r['plc_samples'] == 0 for r in diagnostic['receivers']), 'loopback media loss or concealment'
    assert all(r['packets']['timestamp_step_errors'] == 0 for r in diagnostic['receivers']), 'RTP time step changed'
    assert all(0 <= q <= 8160 for q in diagnostic['queues']), 'PCM water exceeds capacity'
    assert all(abs(ppm) <= 1000 for ppm in diagnostic['drift_ppm']), 'drift correction exceeds limit'
    assert not diagnostic['errors'], 'Hub worker error'
    assert all(w['scheduling']['configured'] and w['scheduling']['policy'] == 1 and w['scheduling']['base_priority'] == 47 for w in diagnostic['media_workers']), 'media worker scheduling did not take effect'
    assert all(r['native_scheduling']['failed'] == 0 and r['native_scheduling']['min_base_priority'] == 47 for r in diagnostic['receivers']), 'native scheduling did not take effect'


try:
    controls = DeviceControls(args.device.removeprefix('coreaudio:'))
    original_rate = controls.rate()
    controls.set_rate(48000)
    subprocess.run([str(hub_bin), 'init', '--directory', str(lab), '--output', args.device], check=True, stdout=subprocess.DEVNULL)
    credential = json.loads((lab / 'admin.json').read_text())
    context = ssl.create_default_context(cadata=credential['certificate'])
    with socket.socket() as free:
        free.bind(('127.0.0.1', 0))
        port = free.getsockname()[1]
    hub = start('hub', [str(hub_bin), 'serve', '--config', str(lab / 'server.json'), '--listen', f'127.0.0.1:{port}'])
    for _ in range(80):
        try:
            get('/v1/hub')
            break
        except OSError:
            time.sleep(.1)
    awake = subprocess.Popen(['/usr/bin/caffeinate', '-i', '-w', str(hub.pid)])
    children.append(awake)
    for name, frequency in [('sender-a', 437), ('sender-b', 659)]:
        start(name, [str(hub_bin), 'send', '--credential', str(lab / (name + '.json')), '--hub', f'https://localhost:{port}', '--seconds', str(args.seconds + 20), '--frequency', str(frequency)])
    time.sleep(3)
    capture = start('capture', [str(audio), 'capture', '--device', args.device, '--rate', '48000', '--period', '256', '--seconds', str(args.seconds)])
    warmup = get('/v1/diagnostics')
    report['warmup_diagnostic'] = warmup
    validate_diagnostic(warmup)
    began = time.monotonic()
    samples = []
    next_report = began
    while capture.poll() is None:
        for name, child in zip(['hub', 'caffeinate', 'sender-a', 'sender-b'], children[:4]):
            if child.poll() is not None:
                raise RuntimeError(f'{name} exited early: {child.returncode}')
        now = time.monotonic()
        if now >= next_report:
            diagnostic = get('/v1/diagnostics')
            report['last_diagnostic']=diagnostic
            resources = {name: process_stats.sample(pid) for name, pid in named_processes.items()}
            sample = {'elapsed_seconds': round(now - began, 2), 'rss_kib': resources['hub']['rss_bytes'] // 1024, 'resources': resources, 'diagnostic': diagnostic}
            samples.append(sample)
            (out / 'samples.jsonl').open('a').write(json.dumps(sample) + '\n')
            validate_diagnostic(diagnostic)
            print(json.dumps({'elapsed_seconds': sample['elapsed_seconds'], 'output_frames': diagnostic['output_frames'], 'underrun_frames': diagnostic['underrun_frames'], 'rss_kib': sample['rss_kib']}), flush=True)
            next_report += 30
        if now - began > args.seconds + 20:
            raise TimeoutError('native capture exceeded duration')
        time.sleep(.2)
    assert capture.returncode == 0
    captured = read_rows('capture')
    final = next(r for r in captured if r['event'] == 'capture_complete')
    assert final['stats']['errors'] == final['stats']['callback_over_budget'] == final['stats']['dropped_frames'] == final['stats']['stale_frames'] == 0
    assert final['stats']['frames'] >= args.seconds * 48000 * .99
    windows = [r['stats'] for r in captured if r['event'] == 'capture_stats'][2:]
    fractions = [(b['silent_frames'] - a['silent_frames']) / (b['frames'] - a['frames']) for a, b in zip(windows, windows[1:]) if b['frames'] > a['frames']]
    assert fractions and max(fractions) < .001, 'steady output contains a gap hidden by silence'
    warm = samples[2:]
    if len(warm) >= 5:
        assert max(s['rss_kib'] for s in warm) - min(s['rss_kib'] for s in warm) < 20000, 'unexpected memory growth'
    final_diagnostic = get('/v1/diagnostics')
    report['last_diagnostic'] = final_diagnostic
    validate_diagnostic(final_diagnostic)
    assert all(child.poll() is None for child in children[:4]), 'Hub or Sender exited early'
    sender_summaries = {}
    for name in ['sender-a', 'sender-b']:
        latest = [r for r in read_rows(name) if r['event'] == 'sender_stats'][-1]
        assert latest['queue_drops'] == latest['audio_queue_dropped'] == latest['skipped_source_periods'] == 0, 'Sender timeline or packet queue interrupted'
        assert latest['media_scheduling']['configured'] and latest['media_scheduling']['base_priority'] == 47
        sender_summaries[name] = latest
    report.update(passed=True, capture=final, final_diagnostic=final_diagnostic, senders=sender_summaries, sample_count=len(samples), max_silent_fraction=max(fractions), max_rss_kib=max(s['rss_kib'] for s in samples), min_rss_kib=min(s['rss_kib'] for s in samples))
    resource_summary = {}
    for name in named_processes:
        readings = [sample['resources'][name] for sample in (warm or samples)]
        first, last = readings[0], readings[-1]
        resource_summary[name] = {
            'cpu_percent_one_core_mean': 100 * (last['cpu_ns'] - first['cpu_ns']) / max(1, last['monotonic_ns'] - first['monotonic_ns']),
            'cpu_percent_one_core_peak_sample': max(r['cpu_percent_one_core'] or 0 for r in readings),
            **{f'{key}_{bound}': operation(r[key] for r in readings)
               for key in ('rss_bytes', 'physical_footprint_bytes', 'threads', 'file_descriptors')
               for bound, operation in [('min', min), ('max', max)]},
        }
    report['resources'] = resource_summary
except (Exception, KeyboardInterrupt) as error:
    report['error'] = repr(error)
    # Capture before stopping the Hub; teardown must not overwrite the original
    # fault with Connection refused errors caused by this probe's own cleanup.
    if 'port' in globals() and 'context' in globals():
        try:
            report['failure_diagnostic'] = get('/v1/diagnostics')
        except Exception as diagnostic_error:
            report['failure_diagnostic_error'] = repr(diagnostic_error)
    report['process_status_before_cleanup'] = [c.poll() for c in children]
    report['senders_at_failure'] = {}
    for name in ['sender-a', 'sender-b']:
        if (out / (name + '.jsonl')).exists():
            latest = [r for r in read_rows(name) if r.get('event') == 'sender_stats']
            report['senders_at_failure'][name] = {
                'last_periodic_stats': latest[-1] if latest else None,
                'stderr_before_cleanup': (out / (name + '.stderr')).read_text()[-8192:],
            }
finally:
    cleanup_errors = []
    # Capture and Senders stop before their Hub, preserving causal error logs.
    for child in reversed(children):
        if child.poll() is None:
            child.send_signal(signal.SIGINT)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
    for handle in handles:
        handle.close()
    if controls is not None and original_rate is not None:
        try:
            controls.set_rate(original_rate)
        except Exception as error:
            cleanup_errors.append('rate restore: ' + repr(error))
    try:
        shutil.rmtree(lab)
    except Exception as error:
        cleanup_errors.append('lab cleanup: ' + repr(error))
    if cleanup_errors:
        report['cleanup_errors'] = cleanup_errors
        report['passed'] = False
    (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'evidence': str(out), 'error': report.get('error')}, indent=2))
raise SystemExit(0 if report['passed'] else 1)
