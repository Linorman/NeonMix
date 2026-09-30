#!/usr/bin/env python3
"""Explicit-device macOS capture -> conversion -> SRTP -> native output probe."""
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

ROOT = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--capture', required=True)
p.add_argument('--output', required=True)
args = p.parse_args()
if sys.platform != 'darwin' or args.capture == args.output:
    p.error('macOS and different explicit capture/output devices are required')
hub_bin, audio = ROOT / 'target/release/neonmix-hub', ROOT / 'target/release/neonmix-audio'
out = ROOT / 'artifacts/capture-sender' / time.strftime('%Y%m%d-%H%M%S')
out.mkdir(parents=True)
lab = Path(tempfile.mkdtemp(prefix='capture-sender-', dir=ROOT / '.local/tmp'))
children, handles = [], []
report = {'capture': args.capture, 'output': args.output, 'passed': False, 'cases': []}


def start(name, argv):
    log, err = (out / (name + '.jsonl')).open('w'), (out / (name + '.stderr')).open('w')
    handles.extend([log, err])
    child = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=err)
    children.append(child)
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


try:
    subprocess.run([str(hub_bin), 'init', '--directory', str(lab), '--output', args.output], check=True, stdout=subprocess.DEVNULL)
    credential = json.loads((lab / 'admin.json').read_text())
    context = ssl.create_default_context(cadata=credential['certificate'])
    with socket.socket() as free:
        free.bind(('127.0.0.1', 0))
        port = free.getsockname()[1]
    hub = start('hub', [str(hub_bin), 'serve', '--config', str(lab / 'server.json'), '--listen', f'127.0.0.1:{port}'])
    for _ in range(60):
        try:
            get('/v1/hub')
            break
        except OSError:
            time.sleep(.1)
    for rate in (44100, 48000, 96000):
        source = start(f'source-{rate}', [str(audio), 'play', '--device', args.capture, '--rate', str(rate), '--period', '480', '--seconds', '8', '--frequency', '437', '--gain-db=-36'])
        time.sleep(.3)
        sender = start(f'sender-{rate}', [str(hub_bin), 'send', '--credential', str(lab / 'sender-a.json'), '--hub', f'https://localhost:{port}', '--capture', args.capture, '--seconds', '6'])
        time.sleep(4)
        diagnostic = get('/v1/diagnostics')
        assert len(diagnostic['receivers']) == 1
        receive = diagnostic['receivers'][0]
        assert receive['authenticated'] and receive['pcm_frames'] > 48000 and receive['pcm_peak'] > .01
        assert receive['packets']['timestamp_step_errors'] == receive['queue_drops'] == 0
        assert diagnostic['output_stats']['errors'] == diagnostic['output_stats']['callback_over_budget'] == 0
        assert sender.wait(timeout=8) == source.wait(timeout=8) == 0
        rows = [json.loads(line) for line in (out / f'sender-{rate}.jsonl').read_text().splitlines()]
        opened = next(r for r in rows if r['event'] == 'sender_started')
        assert opened['capture']['format']['sample_rate'] == rate
        report['cases'].append({'native_rate': rate, 'sender': opened, 'diagnostic': diagnostic})
        print(json.dumps({'native_rate': rate, 'passed': True}), flush=True)
    report['passed'] = True
except Exception as error:
    report['error'] = str(error)
finally:
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
    shutil.rmtree(lab)
    (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'error': report.get('error'), 'evidence': str(out)}, indent=2))
raise SystemExit(0 if report['passed'] else 1)
