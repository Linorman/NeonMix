#!/usr/bin/env python3
"""macOS real-device AirPlay fixture with immutable project-local binaries.

Run through tools/dev. Status never prints PINs or credentials. Start/stop retain
the receiver identity, pairings and .credentials; explicit clean removes only
this manifest-owned fixture directory. Legacy profiles require migration.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import socket
import struct
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / '.local/airplay/manual-session.json'


def file_profiles(state):
    """Check format before starting or replacing an existing receiver's code."""
    state = Path(state).resolve()
    if state.parent != ROOT / '.local/airplay' or not state.name.startswith('manual-'):
        raise RuntimeError('refusing unowned fixture path')
    for relative, version in [('hub/server.json', 2), ('hub/admin.json', 2),
                              ('hub/airplay/receiver.json', 1)]:
        path = state / relative
        if not path.exists():
            if relative.endswith('receiver.json'):
                continue
            raise RuntimeError('saved receiver profile missing; refusing to recreate identity')
        data = json.loads(path.read_text())
        if data.get('version') != version or data.get('credential_store') != 'file':
            raise RuntimeError('migration_required: migrate the saved fixture before starting or replacing binaries')
    return state


def request(state, body):
    payload = json.dumps({'version': 1, 'request': body}).encode()
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(35)
        stream.connect(str(state / 'ipc.sock'))
        stream.sendall(struct.pack('>I', len(payload)) + payload)
        def exact(length):
            result = bytearray()
            while len(result) < length:
                data = stream.recv(length - len(result))
                if not data:
                    raise RuntimeError('fixture IPC closed')
                result.extend(data)
            return result
        length = struct.unpack('>I', exact(4))[0]
        if length > 262144:
            raise RuntimeError('fixture IPC exceeded limit')
        reply = json.loads(exact(length))
        if not reply['ok']:
            raise RuntimeError('fixture request failed: ' + body['type'])
        return reply['data']


def airplay(state, command=None):
    return request(state, {'type': 'airplay', 'credential': 'hub/admin.json',
                           'hub': None, 'command': command})


def status(state):
    raw = airplay(state)
    fields = ['enabled', 'ready', 'active', 'revision', 'error', 'failure_stage',
              'received_blocks', 'released_blocks', 'rejected_blocks', 'ingress',
              'format', 'playback_mode', 'media_resets', 'last_media_reset',
              'pairing_window_open', 'pairing_window_remaining_seconds']
    value = {key: raw.get(key) for key in fields}
    value['pairing_pin_available'] = bool(raw.get('pairing_pin'))
    diagnostics = request(state, {'type': 'diagnostics', 'credential': 'hub/admin.json', 'hub': None})
    if isinstance(diagnostics, list):
        diagnostics = diagnostics[-1]
    if isinstance(diagnostics, dict) and isinstance(diagnostics.get('remote'), dict):
        diagnostics = diagnostics['remote']
    if isinstance(diagnostics, dict):
        value['diagnostics'] = {key: diagnostics.get(key) for key in
                               ['output_frames', 'timed_late_frames', 'timed_drift_ppm',
                                'timed_phase_error_ns', 'meters', 'callback_errors',
                                'callback_over_budget', 'output_underruns']}
    return value


def stop(clean=False):
    if not MANIFEST.exists():
        return
    manifest = json.loads(MANIFEST.read_text())
    state = Path(manifest['state_dir']).resolve()
    if state.parent != ROOT / '.local/airplay' or not state.name.startswith('manual-'):
        raise RuntimeError('refusing unowned fixture path')
    if clean:
        file_profiles(state)
    # Terminate only verified manifest-owned processes directly.
    pids = {manifest.get('background_pid'), manifest.get('ui_pid')}
    for pid in tuple(pids):
        if pid:
            children = subprocess.run(['pgrep', '-P', str(pid)], capture_output=True, text=True)
            pids.update(int(p) for p in children.stdout.split())
    for pid in pids:
        if not pid:
            continue
        command = subprocess.run(['ps', '-p', str(pid), '-o', 'command='], capture_output=True, text=True).stdout
        if str(state) in command:
            os.kill(pid, signal.SIGTERM)
    time.sleep(0.5)
    if not clean:
        manifest.update(background_pid=None, ui_pid=None)
        MANIFEST.write_text(json.dumps(manifest))
        return
    shutil.rmtree(state)
    MANIFEST.unlink()


def start(name=None):
    if MANIFEST.exists():
        manifest = json.loads(MANIFEST.read_text())
        state = file_profiles(manifest['state_dir'])
        # Repeated start is idempotent, including after PIN expiry: no implicit
        # reset of the receiver, pairing window, or active Apple source.
        if (state / 'ipc.sock').exists():
            current = status(state)
            if name is None or request(state, {'type': 'status'})['hub_settings']['name'] == name:
                return current
            return restart(name)
        return launch_existing(state, name)
    name = name or 'AirPlay 复测'
    state = ROOT / '.local/airplay' / ('manual-stable-' + uuid.uuid4().hex[:8])
    state.mkdir(mode=0o700, parents=True)
    binaries = state / 'bin'
    binaries.mkdir()
    for binary_name in ['neonmix-hub', 'neonmix-airplay-worker', 'neonmix-background',
                 'neonmix-audio', 'neonmix-desktop']:
        shutil.copy2(ROOT / 'target/release' / binary_name, binaries / binary_name)
    def launch(name, log):
        env = os.environ.copy()
        env['NEONMIX_AIRPLAY_TRACE'] = '1'
        with (state / log).open('w') as handle:
            return subprocess.Popen([str(binaries / name), '--state-dir', str(state)],
                                    stdout=handle, stderr=handle, cwd=ROOT, env=env,
                                    start_new_session=True)
    background = launch('neonmix-background', 'background.log')
    manifest = {'state_dir': str(state), 'background_pid': background.pid, 'ui_pid': None}
    MANIFEST.write_text(json.dumps(manifest))
    for _ in range(100):
        if (state / 'ipc.sock').exists():
            break
        if background.poll() is not None:
            raise RuntimeError('fixture background exited')
        time.sleep(0.05)
    request(state, {'type': 'hub_setup', 'settings': {'name': name,
                                                    'output': 'coreaudio:BuiltInSpeakerDevice'}})
    settings = request(state, {'type': 'status'})['hub_settings']
    if settings['name'] != name:
        raise RuntimeError('fixture room name differs from requested name')
    request(state, {'type': 'hub_start'})
    for _ in range(100):
        try:
            current = airplay(state)
            break
        except (OSError, RuntimeError):
            time.sleep(0.1)
    else:
        raise RuntimeError('fixture Hub not ready')
    airplay(state, {'expected_revision': current['revision'], 'operation': {'action': 'enable'}})
    for _ in range(100):
        current = airplay(state)
        if current['ready']:
            break
        time.sleep(0.1)
    else:
        raise RuntimeError('fixture Speaker not ready')
    ui = launch('neonmix-desktop', 'ui.log')
    manifest['ui_pid'] = ui.pid
    MANIFEST.write_text(json.dumps(manifest))
    return status(state)


def launch_existing(state, name=None):
    state = file_profiles(state)
    def launch(binary_name, log):
        env = os.environ.copy()
        env['NEONMIX_AIRPLAY_TRACE'] = '1'
        with (state / log).open('a') as handle:
            return subprocess.Popen([str(state / 'bin' / binary_name), '--state-dir', str(state)],
                                    stdout=handle, stderr=handle, cwd=ROOT, env=env,
                                    start_new_session=True)
    background = launch('neonmix-background', 'background.log')
    manifest = {'state_dir': str(state), 'background_pid': background.pid, 'ui_pid': None}
    MANIFEST.write_text(json.dumps(manifest))
    for _ in range(100):
        if (state / 'ipc.sock').exists():
            break
        if background.poll() is not None:
            raise RuntimeError('fixture background exited')
        time.sleep(.05)
    if name is not None:
        settings = request(state, {'type': 'status'})['hub_settings']
        request(state, {'type': 'hub_settings', 'settings': dict(settings, name=name)})
    request(state, {'type': 'hub_start'})
    for _ in range(100):
        try:
            current = airplay(state)
            break
        except (OSError, RuntimeError):
            time.sleep(.1)
    else:
        raise RuntimeError('fixture Hub not ready')
    airplay(state, {'expected_revision': current['revision'], 'operation': {'action': 'enable'}})
    for _ in range(100):
        if airplay(state)['ready']:
            break
        time.sleep(.1)
    else:
        raise RuntimeError('fixture Speaker not ready')
    ui = launch('neonmix-desktop', 'ui.log')
    manifest['ui_pid'] = ui.pid
    MANIFEST.write_text(json.dumps(manifest))
    return status(state)


def restart(name=None, refresh_hub=False, refresh_worker=False, refresh_desktop=False):
    """Keep this receiver's identity and file keys while enabling fixed trace."""
    manifest = json.loads(MANIFEST.read_text())
    state = file_profiles(manifest['state_dir'])
    current = airplay(state)
    airplay(state, {'expected_revision': current['revision'], 'operation': {'action': 'disable'}})
    request(state, {'type': 'hub_stop'})
    if name is not None:
        settings = request(state, {'type': 'status'})['hub_settings']
        request(state, {'type': 'hub_settings', 'settings': dict(settings, name=name)})
    request(state, {'type': 'shutdown'})
    if manifest.get('ui_pid'):
        command = subprocess.run(['ps', '-p', str(manifest['ui_pid']), '-o', 'command='],
                                 capture_output=True, text=True).stdout
        if str(state) in command:
            os.kill(manifest['ui_pid'], signal.SIGTERM)
    for _ in range(100):
        if not (state / 'ipc.sock').exists():
            break
        time.sleep(.05)
    if refresh_hub:
        # HubStop waits for the old child, so its executable is no longer mapped
        # when we replace it. Receiver ID and .credentials remain intact.
        shutil.copy2(ROOT / 'target/release/neonmix-hub', state / 'bin/neonmix-hub')
    if refresh_worker:
        shutil.copy2(ROOT / 'target/release/neonmix-airplay-worker', state / 'bin/neonmix-airplay-worker')
    if refresh_desktop:
        # Never replace a running UI executable; keep receiver/store files intact.
        for _ in range(100):
            pid = manifest.get('ui_pid')
            command = subprocess.run(['ps', '-p', str(pid), '-o', 'command='], capture_output=True, text=True).stdout if pid else ''
            if str(state) not in command:
                break
            time.sleep(.05)
        else:
            raise RuntimeError('fixture UI did not stop; refusing executable replacement')
        shutil.copy2(ROOT / 'target/release/neonmix-desktop', state / 'bin/neonmix-desktop')
    return launch_existing(state)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['start', 'status', 'stop', 'restart', 'clean'])
    parser.add_argument('--name', help='Keep the saved room name when omitted')
    parser.add_argument('--refresh-hub', action='store_true')
    parser.add_argument('--refresh-worker', action='store_true')
    parser.add_argument('--refresh-desktop', action='store_true')
    args = parser.parse_args()
    assert platform.system() == 'Darwin', 'macOS only'
    if args.action == 'start':
        value = start(args.name)
    elif args.action == 'restart':
        value = restart(args.name, args.refresh_hub, args.refresh_worker, args.refresh_desktop)
    elif args.action in ['stop', 'clean']:
        stop(clean=args.action == 'clean')
        value = {'stopped': True, 'identity_retained': args.action != 'clean'}
    else:
        value = status(Path(json.loads(MANIFEST.read_text())['state_dir']))
    print(json.dumps(value, ensure_ascii=False))


if __name__ == '__main__':
    main()
