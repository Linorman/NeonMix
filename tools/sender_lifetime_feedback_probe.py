#!/usr/bin/env python3
"""Actual TLS Hub/Sender duration, explicit stop and pre-capture feedback checks."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--device', required=True, help='Explicit virtual output; never changes the default route')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT):
        parser.error('output must stay inside the project')
    output.mkdir(parents=True, exist_ok=True)
    binary = ROOT / 'target/release/neonmix-hub'
    lab = Path(tempfile.mkdtemp(prefix='p09-sender-', dir=ROOT / '.local/tmp'))
    children, handles = [], []
    report = {'passed': False, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'scope': 'real TLS/media pump and explicit virtual output; no physical speaker or 24h soak', 'checks': {}}
    try:
        subprocess.run([str(binary), 'init', '--directory', str(lab), '--output', args.device], check=True, stdout=subprocess.DEVNULL)

        def spawn(name, command, pipe=False):
            log = (output / (name + '.jsonl')).open('w'); handles.append(log)
            error = (output / (name + '.stderr')).open('w'); handles.append(error)
            child = subprocess.Popen([str(binary), *command], cwd=ROOT, stdin=subprocess.PIPE if pipe else subprocess.DEVNULL,
                stdout=log, stderr=error)
            children.append(child)
            return child

        def events(name):
            return [json.loads(line) for line in (output / (name + '.jsonl')).read_text().splitlines() if line.startswith('{')]

        def wait_event(child, name, kind, seconds=12):
            until = time.monotonic() + seconds
            while child.poll() is None and time.monotonic() < until:
                if any(event.get('event') == kind for event in events(name)):
                    return
                time.sleep(.05)
            raise RuntimeError(name + ' did not publish ' + kind)

        def stop(child):
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
            assert child.wait(timeout=10) == 0

        def start_hub(listen):
            with socket.socket(socket.AF_INET6 if ':' in listen else socket.AF_INET) as free:
                free.bind((listen, 0)); port = free.getsockname()[1]
            address = f'[{listen}]:{port}' if ':' in listen else f'{listen}:{port}'
            child = spawn('hub-' + str(port), ['serve', '--config', str(lab / 'server.json'), '--listen', address])
            wait_event(child, 'hub-' + str(port), 'hub_started')
            return child, port

        hub, port = start_hub('0.0.0.0')
        base = ['send', '--credential', str(lab / 'sender-a.json')]
        addresses = [('loopback', '127.0.0.1')]
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as route:
            route.connect(('192.0.2.1', 9))
            local = route.getsockname()[0]
            if not local.startswith('127.'):
                addresses.append(('lan', local))
        for label, address in addresses:
            name = 'feedback-' + label
            sender = spawn(name, [*base, '--hub', f'https://{address}:{port}', '--capture', args.device, '--seconds', '1'])
            assert sender.wait(timeout=12) != 0
            assert 'local_feedback_loop' in (output / (name + '.stderr')).read_text()
            assert not any(event.get('event') in ['sender_started', 'capture_closed'] for event in events(name))
            report['checks'][label + '_same_endpoint_rejected_before_capture'] = True
        url = f'https://127.0.0.1:{port}'
        timed = spawn('duration', [*base, '--hub', url, '--seconds', '1'])
        assert timed.wait(timeout=15) == 0
        assert any(event.get('event') == 'sender_stopped' and event.get('reason') == 'duration_elapsed' for event in events('duration'))
        continuous = spawn('continuous', [*base, '--hub', url, '--until-stopped', '--managed-control-stdin'], True)
        wait_event(continuous, 'continuous', 'sender_stats')
        assert continuous.poll() is None
        started = next(event for event in events('continuous') if event['event'] == 'sender_started')
        assert started['sender_target']['hub_id'] is not None
        assert started['sender_target']['room_name'] is not None
        continuous.stdin.write(b'{"version":1,"type":"stop"}\n'); continuous.stdin.flush()
        assert continuous.wait(timeout=10) == 0
        assert any(event.get('event') == 'sender_stopped' and event.get('reason') == 'user_stopped' for event in events('continuous'))
        report['checks']['actual_duration_and_continuous_stop_reasons'] = True
        stop(hub)
        hub, port = start_hub('::1')
        sender = spawn('feedback-ipv6', [*base, '--hub', f'https://[::1]:{port}', '--capture', args.device, '--seconds', '1'])
        assert sender.wait(timeout=12) != 0
        assert 'local_feedback_loop' in (output / 'feedback-ipv6.stderr').read_text()
        assert not any(event.get('event') in ['sender_started', 'capture_closed'] for event in events('feedback-ipv6'))
        report['checks']['ipv6_same_endpoint_rejected_before_capture'] = True
        stop(hub)
        report['passed'] = True
    except Exception as error:
        report['error'] = str(error)
    finally:
        for child in children:
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill(); child.wait()
            if child.stdin:
                child.stdin.close()
        for handle in handles:
            handle.close()
        shutil.rmtree(lab)
        report['all_owned_children_exited'] = all(child.poll() is not None for child in children)
        report['private_fixture_removed'] = not lab.exists()
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
