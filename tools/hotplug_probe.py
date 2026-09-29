#!/usr/bin/env python3
"""Operator-assisted, silent physical-output removal/reconnect probe. Never reroutes."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--device', required=True, help='Explicit stable output ID')
parser.add_argument('--seconds', type=int, default=180)
parser.add_argument('--rate', type=int, default=48000)
parser.add_argument('--period', type=int, default=256)
args = parser.parse_args()
if not 10 <= args.seconds <= 3600:
    parser.error('--seconds must be between 10 and 3600')
binary = ROOT / 'target/release/neonmix-audio'
if __import__('sys').platform == 'win32':
    binary = binary.with_suffix('.exe')
out = ROOT / 'artifacts/hotplug' / time.strftime('%Y%m%d-%H%M%S')
out.mkdir(parents=True, exist_ok=True)
play = [str(binary), 'play', '--device', args.device, '--rate', str(args.rate),
        '--period', str(args.period), '--signal', 'silence']
children, handles = [], []


def launch(name, command):
    stdout = (out / f'{name}.jsonl').open('w')
    stderr = (out / f'{name}.stderr').open('w')
    handles.extend([stdout, stderr])
    child = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
    children.append(child)
    return child


def rows(name):
    result = []
    for line in (out / f'{name}.jsonl').read_text().splitlines():
        try:
            result.append(json.loads(line))
        except json.JSONDecodeError:
            pass  # A concurrent final line may still be incomplete.
    return result


report = {'device_id': args.device, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
          'scope': 'physical removal and fresh-process reopen; not same-process epoch allocation',
          'passed': False}
try:
    watch = launch('watch', [str(binary), 'watch', '--seconds', str(args.seconds)])
    output = launch('output', play + ['--seconds', str(args.seconds)])
    deadline = time.monotonic() + args.seconds
    ready = removed = returned = False
    while time.monotonic() < deadline:
        events = rows('watch')
        removed = any(e['event'] == 'removed' and e.get('device_id') == args.device for e in events)
        saw_removed = False
        for event in events:
            if event['event'] == 'removed' and event.get('device_id') == args.device:
                saw_removed = True
            if saw_removed and event['event'] == 'added' and event['device']['id'] == args.device:
                returned = True
        live = rows('output')
        if not ready and any(e['event'] == 'output_stats' and e.get('clock') for e in live):
            ready = True
            print(f'READY: silent output running; unplug, wait 3 seconds, reconnect. Logs: {out}', flush=True)
        if returned or watch.poll() is not None or (output.poll() is not None and not ready):
            break
        time.sleep(0.2)
    report.update(ready=ready, removed=removed, returned_same_id=returned)
    if returned:
        try:
            output.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        report['original_exit_code'] = output.poll()
        faults = [e for e in rows('output') if e['event'] == 'output_fault']
        report['fault'] = faults[-1] if faults else None
        # Do not overlap a new stream if the old one failed to stop on removal.
        if output.poll() is not None and output.returncode != 0:
            reopened = launch('reopened', play + ['--seconds', '3'])
            reopened.wait(timeout=15)
            complete = next((e for e in rows('reopened') if e['event'] == 'output_complete'), None)
            report['reopened'] = complete
            report['reopened_exit_code'] = reopened.returncode
            report['passed'] = bool(ready and removed and faults and complete and reopened.returncode == 0
                                    and complete['info']['device_id'] == args.device
                                    and complete['stats']['errors'] == 0
                                    and complete['stats']['callback_over_budget'] == 0
                                    and complete['clock'] and complete['clock']['native_stream_frames'] > 0)
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
    (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2), flush=True)
raise SystemExit(0 if report['passed'] else 1)
