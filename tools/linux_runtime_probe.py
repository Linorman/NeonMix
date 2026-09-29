#!/usr/bin/env python3
"""Exercise E01 on a running Linux PipeWire user session, without changing default routing.
Creates only a uniquely named temporary NeonMix sink and removes it afterwards.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/neonmix-audio')
args = parser.parse_args()
binary = args.binary.resolve()
folder = ROOT / 'artifacts/linux-runtime'
folder.mkdir(parents=True, exist_ok=True)
room = f'e01-{os.getpid()}'
device = f'pipewire:neonmix.sink.{room}'
children = []
handles = []
results = {}


def start(name, argv):
    output = (folder / f'{name}.jsonl').open('w')
    error = (folder / f'{name}.stderr').open('w')
    handles.extend([output, error])
    child = subprocess.Popen([str(binary), *argv], stdout=output, stderr=error)
    children.append(child)
    return child


def rows(name):
    content = (folder / f'{name}.jsonl').read_text()
    return [json.loads(line) for line in content.splitlines() if line.strip()]


def checked(command):
    return subprocess.run(command, check=True, capture_output=True, text=True, timeout=30).stdout


def wait_for(condition, seconds=15):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        if condition():
            return
        time.sleep(0.1)
    raise TimeoutError('Timed out waiting for a native audio state transition')


try:
    baseline = json.loads(checked([str(binary), 'devices']))
    results['empty_or_existing_devices'] = {'passed': True, 'count': len(baseline)}
    sink = start('sink', ['sink', '--room', room, '--seconds', '120'])
    wait_for(lambda: any(row['event'] == 'sink_ready' for row in rows('sink')))
    devices = json.loads(checked([str(binary), 'devices']))
    (folder / 'devices.json').write_text(json.dumps(devices, indent=2)+'\n')
    selected = next(item for item in devices if item['id'] == device)
    assert selected['output'] is not None and selected['input'] is not None
    results['enumeration'] = {'passed': True, 'device': selected}

    rates = {config['sample_rate'] for config in selected['supported_output']}
    rates &= {config['sample_rate'] for config in selected['supported_input']}
    for rate in [48000, 44100]:
        if rate not in rates:
            results[f'bridge_{rate}'] = {'passed': False, 'reason': 'rate not advertised by this PipeWire graph'}
            continue
        probe = subprocess.run([sys.executable, str(ROOT/'tools/probe.py'), '--binary', str(binary),
                                '--device', device, '--mode', 'loopback', '--rate', str(rate)],
                               capture_output=True, text=True, timeout=30)
        shutil.copytree(ROOT/'artifacts/virtual-probe', folder/f'bridge-{rate}', dirs_exist_ok=True)
        report = json.loads((folder/f'bridge-{rate}/result.json').read_text())
        results[f'bridge_{rate}'] = report
        assert probe.returncode == 0, probe.stdout + probe.stderr

    mono_room = room + '-mono'
    mono_device = f'pipewire:neonmix.sink.{mono_room}'
    start('mono-sink', ['sink', '--room', mono_room, '--channels', '1', '--seconds', '120'])
    wait_for(lambda: any(row['event'] == 'sink_ready' for row in rows('mono-sink')))
    mono_info = next(item for item in json.loads(checked([str(binary), 'devices'])) if item['id'] == mono_device)
    assert mono_info['output']['channels'] == 1
    for pattern in ['left', 'anti-phase']:
        options = ['--expect-silence'] if pattern == 'anti-phase' else []
        probe = subprocess.run([sys.executable, str(ROOT/'tools/probe.py'), '--binary', str(binary),
                                '--device', mono_device, '--mode', 'loopback', '--rate', '48000',
                                '--channel', pattern, *options], capture_output=True, text=True, timeout=30)
        shutil.copytree(ROOT/'artifacts/virtual-probe', folder/f'mono-{pattern}', dirs_exist_ok=True)
        report = json.loads((folder/f'mono-{pattern}/result.json').read_text())
        results[f'mono_{pattern}'] = report
        assert probe.returncode == 0, probe.stdout + probe.stderr
        if pattern == 'left':
            expected_peak = 10 ** (-36 / 20) / 2
            assert abs(report['measurement']['peak'] - expected_peak) < expected_peak * 0.02

    # Three seconds guarantees a full empty 1 Hz observation window after async pause drains.
    output = start('pause-tone', ['play', '--device', device, '--seconds', '7', '--frequency', '437'])
    capture = start('pause', ['capture', '--device', device, '--mode', 'loopback', '--seconds', '6', '--pause-at', '1', '--resume-at', '4'])
    assert capture.wait(timeout=20) == 0
    assert output.wait(timeout=20) == 0
    pause_rows = rows('pause')
    headers = [r['header'] for r in pause_rows if r['event'] == 'capture_block']
    assert any(h['discontinuity_flags'] & 16 and h['source_sample_position'] == 0 for h in headers)
    final = next(r for r in pause_rows if r['event'] == 'capture_complete')
    assert final['stats']['no_data_intervals'] >= 1, 'no fully empty diagnostic window observed during the three-second pause'
    results['pause_resume'] = {'passed': True, 'final': final}

    registry = json.loads(checked(['pw-dump']))
    node = next(obj['id'] for obj in registry if obj.get('info', {}).get('props', {}).get('node.name') == f'neonmix.sink.{room}')
    output = start('mute-tone', ['play', '--device', device, '--seconds', '7', '--frequency', '437'])
    capture = start('system-mute', ['capture', '--device', device, '--mode', 'loopback', '--seconds', '6'])
    time.sleep(1.5)
    checked(['wpctl', 'set-mute', str(node), '1'])
    time.sleep(2.5)
    checked(['wpctl', 'set-mute', str(node), '0'])
    assert capture.wait(timeout=20) == 0
    assert output.wait(timeout=20) == 0
    stats = [r['stats'] for r in rows('system-mute') if r['event'] == 'capture_stats']
    assert any(b['frames'] > a['frames'] and b['silent_frames']-a['silent_frames'] > 30000 for a,b in zip(stats, stats[1:]))
    assert stats[-1]['frames']-stats[-1]['silent_frames'] > stats[0]['frames']-stats[0]['silent_frames']
    results['system_mute'] = {'passed': True, 'stats': stats}

    watcher = start('watch', ['watch', '--seconds', '6'])
    output = start('removed-output', ['play', '--device', device, '--seconds', '10'])
    capture = start('removed-input', ['capture', '--device', device, '--mode', 'loopback', '--seconds', '10'])
    time.sleep(1.5)
    checked(['pw-cli', 'destroy', str(node)])
    capture_exit = capture.wait(timeout=8)
    output_exit = output.wait(timeout=8)
    assert capture_exit != 0 and output_exit != 0, f'device removal was not reported: capture={capture_exit}, output={output_exit}'
    assert watcher.wait(timeout=12) == 0, (folder/'watch.stderr').read_text()
    assert any(r['event'] == 'removed' and r['device_id'] == device for r in rows('watch')), 'missing removed event'
    results['device_removed'] = {'passed': True, 'capture_exit': capture_exit, 'output_exit': output_exit}
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
    (folder / 'result.json').write_text(json.dumps(results, indent=2)+'\n')
print(json.dumps(results, indent=2))
raise SystemExit(0 if 'failure' not in results and all(r.get('passed') for r in results.values()) else 1)
