#!/usr/bin/env python3
"""Real Linux backgrounds/guardian/PipeWire owner; fixture remote Sender CLI, no PCM."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def request(state, kind, **fields):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(20)
        connection.connect(str(state / ('lifecycle.sock' if kind in ['status', 'shutdown'] else 'ipc.sock')))
        body = json.dumps({'version': 1, 'request': {'type': kind, **fields}}).encode()
        connection.sendall(struct.pack('>I', len(body)) + body)
        def read(count):
            value = b''
            while len(value) < count:
                chunk = connection.recv(count - len(value))
                if not chunk: raise EOFError('IPC closed')
                value += chunk
            return value
        length = struct.unpack('>I', read(4))[0]
        assert length <= 262144
        return json.loads(read(length))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binaries', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binaries, output = args.binaries.resolve(), args.output.resolve()
    if not all(path.is_relative_to(ROOT) for path in [binaries, output]):
        parser.error('binaries and output must stay inside the project')
    output.mkdir(parents=True, exist_ok=True)
    lab = Path(tempfile.mkdtemp(prefix='p09-background-owner-', dir=ROOT / '.local/tmp'))
    children, logs = [], []
    result = {'passed': False, 'scope': 'real Linux IPC/background/guardian/node; fixture Sender CLI, no PCM', 'binaries': {}}
    try:
        install = lab / 'install'; install.mkdir(mode=0o700)
        for name in ['neonmix-background', 'neonmix-guardian', 'neonmix-audio']:
            source = binaries / name
            result['binaries'][name] = hashlib.sha256(source.read_bytes()).hexdigest()
            shutil.copy2(source, install / name); (install / name).chmod(0o755)
        fixture = install / 'neonmix-hub'
        fixture.write_text('#!/bin/sh\ncase "$1" in\nsend) printf \'{"event":"sender_started"}\\n\'; IFS= read -r stop; exit 0;;\n*) printf \'{}\\n\';;\nesac\n')
        fixture.chmod(0o700)
        states = {}
        for label in ['a', 'b']:
            state = lab / label; state.mkdir(mode=0o700); states[label] = state
            for name in ['profiles', 'output']:
                (state / name).mkdir(mode=0o700)
            profile = {'version': 2, 'credential_store': 'file', 'profile_kind': 'member', 'hub_id': str(uuid.uuid4()),
                'certificate': 'fixture-public-certificate', 'secret_ref': str(uuid.uuid4()), 'request_id': str(uuid.uuid4()),
                'name': 'fixture', 'pending': False, 'invitation_id': None, 'device_id': str(uuid.uuid4())}
            binding = {'schema_version': 1, 'revision': 1, 'output_id': str(uuid.uuid4()), 'hub_id': profile['hub_id'],
                'provider': 'neonmix', 'device_id': 'pipewire:neonmix.sink.default', 'display_name': 'NeonMix ' + label,
                'enabled': True, 'authorization_epoch': 1}
            for path, value in [(state / 'profiles/sender.json', profile), (state / 'output/binding.json', binding)]:
                path.write_text(json.dumps(value)); path.chmod(0o600)

        def start(label):
            log = (output / ('background-' + label + '.log')).open('w'); logs.append(log)
            child = subprocess.Popen([str(install / 'neonmix-background'), '--state-dir', str(states[label])], stdout=log, stderr=subprocess.STDOUT)
            children.append(child)
            until = time.monotonic() + 10
            while child.poll() is None and time.monotonic() < until:
                try:
                    if request(states[label], 'status')['ok']:
                        return child
                except OSError:
                    pass
                time.sleep(.05)
            raise RuntimeError('background did not become ready')

        options = {'credential': 'profiles/sender.json', 'output_binding': 'output', 'hub': None}
        a, b = start('a'), start('b')
        assert request(states['a'], 'sender_start', options=options)['ok']
        rejected = request(states['b'], 'sender_start', options=options)
        assert not rejected['ok']
        assert rejected.get('fault', {}).get('code') == 'resource_owned_by_other_instance', rejected
        assert request(states['a'], 'status')['data']['sender']['running']
        assert request(states['b'], 'shutdown')['ok']; assert b.wait(timeout=12) == 0
        assert a.poll() is None and request(states['a'], 'status')['data']['sender']['running']
        result['second_background_rejected_and_its_shutdown_preserved_first'] = True
        assert request(states['a'], 'shutdown')['ok']; assert a.wait(timeout=12) == 0
        b = start('b')
        assert request(states['b'], 'sender_start', options=options)['ok']
        assert request(states['b'], 'shutdown')['ok']; assert b.wait(timeout=12) == 0
        result['restart_acquires_owner_after_first_shutdown'] = True
        with socket.socket(socket.AF_UNIX) as check:
            try:
                check.connect('\0neonmix.virtual-output.owner.' + str(os.getuid()))
            except (FileNotFoundError, ConnectionRefusedError):
                result['kernel_owner_released_after_background_exit'] = True
            else:
                raise RuntimeError('owned virtual output remained after background exit')
        result['passed'] = True
    except Exception as error:
        result['error'] = str(error)
    finally:
        for label, state in locals().get('states', {}).items():
            try:
                request(state, 'shutdown')
            except (OSError, ValueError):
                pass
        for child in children:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=12)
                except subprocess.TimeoutExpired:
                    child.kill(); child.wait()
        for log in logs:
            log.close()
        shutil.rmtree(lab)
        result['all_backgrounds_exited'] = all(child.poll() is not None for child in children)
        result['fixture_removed'] = not lab.exists()
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result), flush=True)
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
