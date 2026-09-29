#!/usr/bin/env python3
"""E01 virtual-device lifecycle checks. Explicit UID, no microphone PCM or default-route changes."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time
from macos_audio_controls import DeviceControls

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--device', required=True)
args = parser.parse_args()
if not args.device.startswith('coreaudio:'):
    parser.error('Use an explicit coreaudio:UID')
binary = ROOT / 'target/release/neonmix-audio'
folder = ROOT / 'artifacts/blackhole/lifecycle' / time.strftime('%Y%m%d-%H%M%S')
folder.mkdir(parents=True, exist_ok=True)
children, handles = [], []
results = {'device_id': args.device, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'cases': {}}
controls = DeviceControls(args.device.removeprefix('coreaudio:'))
original_mute = controls.muted()


def start(name, command, seconds, extra=()):
    out, err = (folder / f'{name}.jsonl').open('w'), (folder / f'{name}.stderr').open('w')
    handles.extend([out, err])
    argv = [str(binary), command, '--device', args.device, '--rate', '48000',
            '--period', '256', '--seconds', str(seconds), *extra]
    child = subprocess.Popen(argv, stdout=out, stderr=err)
    children.append(child)
    return child


def rows(name):
    return [json.loads(line) for line in (folder / f'{name}.jsonl').read_text().splitlines() if line]


def finish(child, name, event):
    code = child.wait(timeout=20)
    events = rows(name)
    final = next((r for r in events if r['event'] == event), None)
    assert code == 0 and final is not None, f'{name}: exit={code}, {events[-1:]}'
    stats = final['stats']
    assert stats['errors'] == 0 and stats['callback_over_budget'] == 0, f'{name}: {stats}'
    assert stats['frames'] > 0
    return events, final


def windows(events):
    stats = [e['stats'] for e in events if e['event'] == 'capture_stats']
    return [{'frames': b['frames'] - a['frames'], 'silent': b['silent_frames'] - a['silent_frames']}
            for a, b in zip(stats, stats[1:])]


def record(name, **evidence):
    results['cases'][name] = {'passed': True, **evidence}
    print(f'{name}: passed', flush=True)


try:
    controls.set_muted(False)
    idle = start('idle', 'capture', 3)
    _, final = finish(idle, 'idle', 'capture_complete')
    assert final['stats']['frames'] == final['stats']['silent_frames']
    assert final['measurement']['peak'] == 0 and final['stats']['no_data_intervals'] == 0
    record('no_source', final=final, interpretation='BlackHole continues delivering silence, not absent callbacks')

    tone = start('pause-tone', 'play', 9, ['--frequency', '437', '--gain-db', '-36'])
    capture = start('pause', 'capture', 7, ['--pause-at', '2', '--resume-at', '5'])
    events, final = finish(capture, 'pause', 'capture_complete')
    finish(tone, 'pause-tone', 'output_complete')
    blocks = [e for e in events if e['event'] == 'capture_block']
    resumed = next(e for e in blocks if e['header']['discontinuity_flags'] & 16)
    assert resumed['header']['source_sample_position'] == 0
    assert resumed['header']['stream_epoch'] > blocks[0]['header']['stream_epoch']
    assert final['stats']['no_data_intervals'] > 0
    assert abs(final['measurement']['estimated_frequency_hz'] - 437) < 1
    record('pause_resume', resumed=resumed, final=final)

    tone = start('mute-tone', 'play', 9, ['--frequency', '437', '--gain-db', '-36'])
    capture = start('system-mute', 'capture', 8)
    time.sleep(2)
    controls.set_muted(True)
    assert controls.muted() == 1
    time.sleep(3)
    controls.set_muted(False)
    assert controls.muted() == 0
    events, final = finish(capture, 'system-mute', 'capture_complete')
    finish(tone, 'mute-tone', 'output_complete')
    intervals = windows(events)
    assert any(w['frames'] > 0 and w['frames'] == w['silent'] for w in intervals)
    assert intervals[0]['frames'] > intervals[0]['silent']
    assert intervals[-1]['frames'] > intervals[-1]['silent']
    record('system_mute', windows=intervals, final=final)

    capture = start('restart', 'capture', 10)
    tone = start('source-first', 'play', 2, ['--frequency', '437', '--gain-db', '-36'])
    finish(tone, 'source-first', 'output_complete')
    time.sleep(3)
    tone = start('source-second', 'play', 3, ['--frequency', '659', '--gain-db', '-36'])
    finish(tone, 'source-second', 'output_complete')
    events, final = finish(capture, 'restart', 'capture_complete')
    intervals = windows(events)
    assert any(w['frames'] > 0 and w['frames'] == w['silent'] for w in intervals[1:5])
    assert any(w['frames'] > w['silent'] + 30000 for w in intervals[5:])
    assert abs(final['measurement']['estimated_frequency_hz'] - 659) < 1, f"restart frequency: {final['measurement']}"
    assert final['stats']['dropped_frames'] == 0 and final['stats']['stale_frames'] == 0
    record('source_stop_restart', windows=intervals, final=final)
except Exception as error:
    results['failure'] = {'type': type(error).__name__, 'message': str(error)}
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
    try:
        controls.set_muted(original_mute)
        results['original_output_mute'] = original_mute
        results['restored_output_mute'] = controls.muted() == original_mute
    except Exception as error:
        results['restore_failure'] = str(error)
    results['passed'] = len(results['cases']) == 4 and 'failure' not in results and results.get('restored_output_mute', False)
    (folder / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
print(json.dumps({'passed': results['passed'], 'folder': str(folder), 'failure': results.get('failure')}, indent=2))
raise SystemExit(0 if results['passed'] else 1)
