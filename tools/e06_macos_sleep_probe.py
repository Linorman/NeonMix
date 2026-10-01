#!/usr/bin/env python3
"""Observe operator sleep/wake, then reopen the exact HAL UID; never schedules sleep."""
import argparse
from datetime import datetime
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--device', default='coreaudio:com.neonmix.audio.virtual-output')
parser.add_argument('--window-seconds', type=int, default=300)
args = parser.parse_args()
if sys.platform != 'darwin' or not 30 <= args.window_seconds <= 600:
    parser.error('macOS and a 30..600 second operator window are required')
binary = ROOT / 'target/release/neonmix-audio'
folder = ROOT / 'artifacts/e06-sleep' / time.strftime('%Y%m%d-%H%M%S')
folder.mkdir(parents=True)
report = {'passed': False, 'device_id': args.device,
          'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
          'scope': 'operator-triggered macOS sleep/wake and exact-UID digital reopen'}
children, handles = [], []
start_wall = time.time()
start_stamp = time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(start_wall))


def rows(name):
    return [json.loads(line) for line in (folder / (name + '.jsonl')).read_text().splitlines() if line.startswith('{')]


def stop(child):
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()


try:
    for operation in ['play', 'capture']:
        stdout, stderr = (folder / (operation + '.jsonl')).open('w'), (folder / (operation + '.stderr')).open('w')
        handles.extend([stdout, stderr])
        child = subprocess.Popen([str(binary), operation, '--device', args.device,
                                  '--rate', '48000', '--period', '256', '--seconds', str(args.window_seconds + 60),
                                  *(['--signal', 'silence'] if operation == 'play' else [])], stdout=stdout, stderr=stderr)
        children.append(child)
    deadline = time.time() + 10
    while not all(any(row.get('stats', {}).get('frames', 0) >= 48000 for row in rows(name)) for name in ['play', 'capture']):
        if time.time() >= deadline or any(child.poll() is not None for child in children):
            raise RuntimeError('Both actual silent streams must be running before sleep')
        time.sleep(.1)
    print('READY: silent HAL input/output active; operator may sleep the Mac and wake it after at least 10 seconds.', flush=True)
    deadline = time.time() + args.window_seconds
    while time.time() < deadline:
        # A fresh power-management record proves actual system sleep; an operator
        # acknowledgement, a wall-clock gap or display-off alone cannot pass.
        power = subprocess.check_output(['pmset', '-g', 'log'], text=True)
        recent = [line for line in power.splitlines() if len(line) >= 19 and line[:19] >= start_stamp and line[:4].isdigit()]
        slept = [line for line in recent if 'Entering Sleep state' in line]
        woke = [line for line in recent if len(line.split()) > 3 and line.split()[3] == 'Wake' and 'Wake from ' in line]
        if slept and woke:
            sleep_time = datetime.strptime(' '.join(slept[-1].split()[:3]), '%Y-%m-%d %H:%M:%S %z').timestamp()
            wake_time = datetime.strptime(' '.join(woke[-1].split()[:3]), '%Y-%m-%d %H:%M:%S %z').timestamp()
            if wake_time - sleep_time < 10:
                time.sleep(2)
                continue
            (folder / 'power-events.log').write_text('\n'.join(recent) + '\n')
            report['sleep_events'] = slept
            report['wake_events'] = woke
            report['sleep_seconds'] = wake_time - sleep_time
            break
        time.sleep(2)
    else:
        raise TimeoutError('No new system sleep and wake records in the operator window')
    for child in children:
        stop(child)
    report['old_streams'] = {name: rows(name)[-2:] for name in ['play', 'capture']}
    query = subprocess.run([str(binary), 'devices'], capture_output=True, text=True, timeout=10)
    query.check_returncode()
    devices = [device for device in json.loads(query.stdout) if device['id'] == args.device]
    assert len(devices) == 1, 'Stable HAL UID missing or duplicated after wake'
    report['restored_device'] = devices[0]
    probe = subprocess.run([sys.executable, str(ROOT / 'tools/probe.py'), '--release', '--device', args.device, '--rate', '48000'], capture_output=True, text=True, timeout=25)
    (folder / 'reopen.log').write_text(probe.stdout + probe.stderr)
    shutil.copytree(ROOT / 'artifacts/virtual-probe', folder / 'reopened-bridge')
    report['reopened_bridge'] = json.loads((folder / 'reopened-bridge/result.json').read_text())
    assert probe.returncode == 0 and report['reopened_bridge']['passed'], 'Digital bridge did not recover after actual wake'
    report['passed'] = True
except Exception as error:
    report['failure'] = {'type': type(error).__name__, 'message': str(error)}
finally:
    for child in children:
        stop(child)
    for handle in handles:
        handle.close()
    (folder / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'failure': report.get('failure'), 'folder': str(folder)}), flush=True)
raise SystemExit(0 if report['passed'] else 1)
