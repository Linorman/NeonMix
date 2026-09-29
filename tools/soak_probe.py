#!/usr/bin/env python3
"""Bounded digital audio stability probe; explicit virtual UID, no default routing."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--device', required=True)
parser.add_argument('--seconds', type=int, default=300)
parser.add_argument('--rate', type=int, choices=[44100, 48000, 96000], default=48000)
parser.add_argument('--period', type=int, default=256)
args = parser.parse_args()
if not 10 <= args.seconds <= 3600:
    parser.error('--seconds must be between 10 and 3600')
binary = ROOT / 'target/release/neonmix-audio'
folder = ROOT / 'artifacts/soak' / time.strftime('%Y%m%d-%H%M%S')
folder.mkdir(parents=True, exist_ok=True)
handles, children = [], []
report = {'device_id': args.device, 'seconds': args.seconds, 'rate': args.rate,
          'period': args.period, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
          'passed': False, 'scope': 'bounded digital-path baseline; not analog or long-term certification'}


def start(name, command, seconds, extra=()):
    out, err = (folder / f'{name}.jsonl').open('w'), (folder / f'{name}.stderr').open('w')
    handles.extend([out, err])
    argv = [str(binary), command, '--device', args.device, '--rate', str(args.rate),
            '--period', str(args.period), '--seconds', str(seconds), *extra]
    child = subprocess.Popen(argv, stdout=out, stderr=err)
    children.append(child)
    return child


def rows(name):
    result = []
    for line in (folder / f'{name}.jsonl').read_text().splitlines():
        try:
            result.append(json.loads(line))
        except json.JSONDecodeError:
            pass  # A concurrent trailing JSON line may not yet be complete.
    return result


try:
    output = start('output', 'play', args.seconds + 2, ['--frequency', '437', '--gain-db', '-36'])
    time.sleep(0.5)
    capture = start('capture', 'capture', args.seconds)
    report['pids'] = {'output': output.pid, 'capture': capture.pid}
    started, next_report = time.monotonic(), 0
    print(f'STARTED {report["pids"]} evidence={folder}', flush=True)
    while capture.poll() is None:
        elapsed = time.monotonic() - started
        if elapsed > args.seconds + 15:
            raise TimeoutError('Capture exceeded its requested duration')
        if output.poll() is not None:
            raise RuntimeError(f'Output stopped before capture: exit={output.returncode}')
        if elapsed >= next_report:
            stats = next((r['stats'] for r in reversed(rows('capture')) if 'stats' in r), {})
            print(json.dumps({'elapsed_seconds': round(elapsed), 'capture_frames': stats.get('frames'),
                              'errors': stats.get('errors'), 'over_budget': stats.get('callback_over_budget')}), flush=True)
            next_report += 30
        time.sleep(0.25)
    output.wait(timeout=10)
    capture_rows, output_rows = rows('capture'), rows('output')
    cap = next((r for r in capture_rows if r['event'] == 'capture_complete'), None)
    out = next((r for r in output_rows if r['event'] == 'output_complete'), None)
    report.update(capture_exit=capture.returncode, output_exit=output.returncode, capture=cap, output=out)
    assert cap and out and capture.returncode == output.returncode == 0, 'missing successful completion'
    for name, final in [('capture', cap), ('output', out)]:
        stats = final['stats']
        assert stats['frames'] >= args.rate * args.seconds * .99, f'{name}: insufficient frames'
        for field in ['errors', 'callback_over_budget', 'dropped_frames', 'stale_frames', 'no_data_intervals']:
            assert stats[field] == 0, f'{name}: {field}={stats[field]}'
    assert abs(cap['measurement']['estimated_frequency_hz'] - 437) < 0.1, 'tone frequency mismatch'
    assert abs(cap['measurement']['peak'] - 10 ** (-36 / 20)) < .00001, 'tone peak mismatch'
    stats = [r['stats'] for r in capture_rows if r['event'] == 'capture_stats'][2:]
    fractions = [(b['silent_frames'] - a['silent_frames']) / (b['frames'] - a['frames'])
                 for a, b in zip(stats, stats[1:]) if b['frames'] > a['frames']]
    assert len(fractions) >= args.seconds - 6, 'insufficient steady observation windows'
    report['max_steady_silent_fraction'] = max(fractions)
    assert max(fractions) < .001, 'unexpected silence in steady capture'
    clocks = [r['clock'] for r in output_rows if r.get('clock')]
    assert len(clocks) >= args.seconds - 1
    assert all(b['clock_timestamp_ns'] > a['clock_timestamp_ns']
               and b['native_device_frame_position'] > a['native_device_frame_position']
               and b['epoch'] == a['epoch'] for a, b in zip(clocks, clocks[1:])), 'clock discontinuity'
    report['passed'] = True
except Exception as error:
    report['failure'] = {'type': type(error).__name__, 'message': str(error)}
finally:
    for child in children:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
    for handle in handles:
        handle.close()
    (folder / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({'passed': report['passed'], 'failure': report.get('failure'), 'folder': str(folder)}), flush=True)
raise SystemExit(0 if report['passed'] else 1)
