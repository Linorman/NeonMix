#!/usr/bin/env python3
"""Real per-UID PipeWire owner identities and two private directories; no PCM."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
ADDRESS = '\0neonmix.virtual-output.owner.' + str(os.getuid())
REQUEST = b'\0{"version":1,"type":"inspect_owner"}'


def inspect():
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(1)
        connection.connect(ADDRESS)
        connection.sendall(REQUEST)
        connection.shutdown(socket.SHUT_WR)
        response = b''
        while chunk := connection.recv(1025 - len(response)):
            response += chunk
            if len(response) > 1024:
                raise RuntimeError('oversized owner response')
        return json.loads(response)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to(ROOT):
        parser.error('output must stay inside the project')
    output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    if not binary.is_relative_to(ROOT):
        parser.error('binary must be inside the project')
    lab = ROOT / '.local/tmp' / ('p09-owner-' + str(uuid.uuid4()))
    lab.mkdir(parents=True, mode=0o700)
    children, logs = [], []
    result = {'platform': 'Linux PipeWire', 'scope': 'real node/owner; no PCM or physical device',
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'passed': False}
    try:
        try:
            inspect()
        except (FileNotFoundError, ConnectionRefusedError):
            pass
        else:
            raise RuntimeError('another owner is already present; left untouched')
        identities = {}
        for label in ('a', 'b'):
            directory = lab / label
            directory.mkdir(mode=0o700)
            state, binding = directory / 'state', directory / 'binding'
            state.mkdir(mode=0o700)
            binding.mkdir(mode=0o700)
            generation, output_id = str(uuid.uuid4()), str(uuid.uuid4())
            identities[label] = (generation, output_id, state, binding)
            path = binding / 'binding.json'
            path.write_text(json.dumps({'schema_version': 1, 'revision': 1, 'output_id': output_id,
                'hub_id': str(uuid.uuid4()), 'provider': 'neonmix', 'device_id': 'pipewire:neonmix.sink.default',
                'display_name': 'NeonMix owner ' + label, 'enabled': True, 'authorization_epoch': 1}))
            path.chmod(0o600)

        def spawn(label, suffix):
            generation, _, state, binding = identities[label]
            log = (output / (suffix + '.log')).open('w'); logs.append(log)
            process = subprocess.Popen([str(binary), 'virtual-output', '--state-directory', str(state),
                '--output-binding', str(binding), '--instance-generation', generation], stdout=log, stderr=subprocess.STDOUT)
            children.append(process)
            return process

        def ready(process, label):
            until = time.monotonic() + 30
            while process.poll() is None and time.monotonic() < until:
                try:
                    owner = inspect()
                    if owner['ready']:
                        assert owner['version'] == 1
                        assert (owner['instance_generation'], owner['output_id']) == identities[label][:2]
                        return owner
                except (FileNotFoundError, ConnectionRefusedError):
                    pass
                time.sleep(.1)
            raise RuntimeError('owner did not become ready')

        a = spawn('a', 'owner-a'); ready(a, 'a')
        b = spawn('b', 'owner-b-rejected'); assert b.wait(timeout=10) != 0
        logs[-1].flush()
        assert 'resource_owned_by_other_instance' in (output / 'owner-b-rejected.log').read_text()
        ready(a, 'a'); assert a.poll() is None
        result['second_directory_rejected_and_first_preserved'] = True
        a.terminate(); assert a.wait(timeout=10) == 0
        b = spawn('b', 'owner-b-after-release'); ready(b, 'b')
        b.terminate(); assert b.wait(timeout=10) == 0
        try:
            inspect()
        except (FileNotFoundError, ConnectionRefusedError):
            result['kernel_owner_released'] = True
        else:
            raise RuntimeError('owner socket remained after stop')
        result['new_instance_can_acquire_after_first_exit'] = True
        result['passed'] = True
    except Exception as error:
        result['error'] = str(error)
    finally:
        for child in children:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill(); child.wait()
        for log in logs:
            log.close()
        shutil.rmtree(lab)
        result['fixture_removed'] = not lab.exists()
        result['all_owned_children_exited'] = all(child.poll() is not None for child in children)
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result), flush=True)
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
