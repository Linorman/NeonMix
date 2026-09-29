#!/usr/bin/env python3
"""Explicit, disruptive Core Audio restart acceptance; requires system administrator confirmation."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--device', required=True)
parser.add_argument('--restart-coreaudio', action='store_true', required=True)
parser.add_argument('--authorization-seconds', type=int, default=60)
args = parser.parse_args()
if sys.platform != 'darwin' or not args.device.startswith('coreaudio:'):
    parser.error('Requires macOS and an explicit Core Audio virtual device UID')
if not 10 <= args.authorization_seconds <= 60:
    parser.error('--authorization-seconds must be between 10 and 60')
binary = ROOT / 'target/release/neonmix-audio'
folder = ROOT / 'artifacts/service-recovery' / time.strftime('%Y%m%d-%H%M%S')
folder.mkdir(parents=True, exist_ok=True)
report = {'device_id': args.device, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
          'authorization_seconds': args.authorization_seconds, 'passed': False}
children, handles = [], []


def start(name, operation):
    out, err = (folder / f'{name}.jsonl').open('w'), (folder / f'{name}.stderr').open('w')
    handles.extend([out, err])
    extra = ['--signal', 'silence'] if operation == 'play' else []
    child = subprocess.Popen([str(binary), operation, '--device', args.device, '--rate', '48000',
                              '--period', '256', '--seconds', str(args.authorization_seconds + 30), *extra], stdout=out, stderr=err)
    children.append(child)
    return child


def rows(name):
    result = []
    for line in (folder / f'{name}.jsonl').read_text().splitlines():
        try:
            result.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return result


try:
    output, capture = start('output', 'play'), start('capture', 'capture')
    deadline = time.monotonic() + 10
    while True:
        ready = all(any(e.get('stats', {}).get('frames', 0) >= 48000 for e in rows(name))
                    for name in ['output', 'capture'])
        if ready:
            break
        if time.monotonic() >= deadline or any(p.poll() is not None for p in children):
            raise RuntimeError('Both silent streams must be active before restarting Core Audio')
        time.sleep(.1)
    print('READY: both streams active; requesting native administrator confirmation to restart Core Audio.', flush=True)
    # If authorization outlives the probe, do not restart after its streams have exited.
    restart = f'/bin/kill -0 {output.pid} && /bin/kill -0 {capture.pid} && /usr/bin/killall coreaudiod'
    authorization = subprocess.run(['/usr/bin/osascript', '-e',
        f'do shell script "{restart}" with administrator privileges'],
        capture_output=True, text=True, timeout=args.authorization_seconds)
    report['restart_exit_code'] = authorization.returncode
    (folder / 'restart.stderr').write_text(authorization.stderr)
    assert authorization.returncode == 0, 'Core Audio restart was not completed'
    for name, child in [('output', output), ('capture', capture)]:
        code = child.wait(timeout=10)
        fault = next((e for e in rows(name) if e['event'] == name + '_fault'), None)
        report[name] = {'exit_code': code, 'fault': fault}
        assert code != 0 and fault and fault['stats']['errors'] > 0, f'{name}: no explicit native failure'
    deadline = time.monotonic() + 15
    while True:
        query = subprocess.run([str(binary), 'devices'], capture_output=True, text=True, timeout=5)
        if query.returncode == 0 and any(d['id'] == args.device for d in json.loads(query.stdout)):
            (folder / 'devices-restored.json').write_text(query.stdout)
            break
        if time.monotonic() > deadline:
            raise TimeoutError('Selected stable UID did not reappear after service restart')
        time.sleep(.5)
    probe = subprocess.run([sys.executable, str(ROOT / 'tools/probe.py'), '--release',
                            '--device', args.device, '--rate', '48000'], capture_output=True, text=True, timeout=25)
    shutil.copytree(ROOT / 'artifacts/virtual-probe', folder / 'reopened-bridge')
    report['reopened_bridge'] = json.loads((folder / 'reopened-bridge/result.json').read_text())
    assert probe.returncode == 0 and report['reopened_bridge']['passed'], 'Reopened digital bridge failed'
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
