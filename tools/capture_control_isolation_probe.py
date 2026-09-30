#!/usr/bin/env python3
"""Native capture remains live while authenticated control retries are delayed."""
import argparse
import http.client
import json
from pathlib import Path
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import time
from lan_fault_gateway import Gateway
from macos_audio_controls import DeviceControls

ROOT = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--capture', required=True)
p.add_argument('--output', required=True)
args = p.parse_args()
if sys.platform != 'darwin' or args.capture == args.output:
    p.error('macOS and different explicit devices required')
binary, audio = ROOT / 'target/release/neonmix-hub', ROOT / 'target/release/neonmix-audio'
out = ROOT / 'artifacts/capture-isolation' / time.strftime('%Y%m%d-%H%M%S')
out.mkdir(parents=True)
lab = Path(tempfile.mkdtemp(prefix='capture-isolation-', dir=ROOT / '.local/tmp'))
controls = DeviceControls(args.capture.removeprefix('coreaudio:'))
original_rate = controls.rate()
children, handles = [], []
gateway = None
report = {'passed': False, 'capture': args.capture, 'output': args.output}


def start(name, argv):
    log, err = (out / (name + '.jsonl')).open('w'), (out / (name + '.stderr')).open('w')
    handles.extend([log, err])
    child = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=err)
    children.append(child)
    return child


def stats():
    rows = []
    for line in (out / 'sender.jsonl').read_text().splitlines():
        try:
            row = json.loads(line)
            if row['event'] == 'sender_stats':
                rows.append(row)
        except ValueError:
            pass
    return rows[-1] if rows else None


try:
    controls.set_rate(48000)
    subprocess.run([str(binary), 'init', '--directory', str(lab), '--output', args.output], check=True, stdout=subprocess.DEVNULL)
    config = json.loads((lab / 'server.json').read_text())
    credential = json.loads((lab / 'admin.json').read_text())
    context = ssl.create_default_context(cadata=credential['certificate'])
    with socket.socket() as free:
        free.bind(('127.0.0.1', 0))
        port = free.getsockname()[1]
    hub = start('hub', [str(binary), 'serve', '--config', str(lab / 'server.json'), '--listen', f'127.0.0.1:{port}'])
    for _ in range(60):
        try:
            c = http.client.HTTPSConnection('localhost', port, context=context, timeout=5)
            c.request('GET', '/v1/hub', headers={'Authorization': 'Bearer ' + credential['token']})
            assert c.getresponse().status == 200
            c.close()
            break
        except OSError:
            time.sleep(.1)
    gateway = Gateway(port, config, lab)
    source = start('source', [str(audio), 'play', '--device', args.capture, '--rate', '48000', '--period', '480', '--seconds', '22', '--frequency', '437', '--gain-db=-36'])
    time.sleep(.3)
    sender = start('sender', [str(binary), 'send', '--credential', str(lab / 'sender-a.json'), '--hub', f'https://localhost:{gateway.port}', '--capture', args.capture, '--seconds', '18'])
    for _ in range(80):
        if stats() and stats()['control_connected']:
            break
        time.sleep(.1)
    before = stats()
    assert before and before['capture_stats']['errors'] == 0
    gateway.snapshot_delay = 6
    gateway.disconnect_events()
    time.sleep(7)
    during = stats()
    assert sender.poll() is None
    assert during['capture_stats']['frames'] - before['capture_stats']['frames'] >= 6 * 48000
    assert during['capture_stats']['errors'] == during['capture_stats']['dropped_frames'] == during['capture_stats']['stale_frames'] == 0
    assert during['sent_packets'] - before['sent_packets'] >= 600
    assert gateway.snapshot_requests > 1
    gateway.snapshot_delay = 0
    for _ in range(100):
        after = stats()
        if after and after['control_subscriptions'] >= 2 and after['control_connected']:
            break
        time.sleep(.1)
    assert after['control_subscriptions'] >= 2
    assert sender.wait(timeout=20) == source.wait(timeout=20) == 0
    report.update(passed=True, before=before, during=during, recovered=after)
except Exception as error:
    report['error'] = repr(error)
finally:
    if gateway:
        gateway.close()
    for child in children:
        if child.poll() is None:
            child.send_signal(signal.SIGINT)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
    for handle in handles:
        handle.close()
    controls.set_rate(original_rate)
    shutil.rmtree(lab)
    (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'error': report.get('error'), 'evidence': str(out)}, indent=2))
raise SystemExit(0 if report['passed'] else 1)
