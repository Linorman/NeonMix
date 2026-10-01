#!/usr/bin/env python3
"""Exercise the actual E06 owner in a project-local, non-root PipeWire session."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--desktop-session', action='store_true')
args = parser.parse_args()
if sys.platform != 'linux' or os.geteuid() == 0:
    raise SystemExit('Requires a non-root Linux user')
OUT = ROOT / 'artifacts/e06-linux' / time.strftime('%Y%m%d-%H%M%S')
OUT.mkdir(parents=True)
runtime = Path('/run/user') / str(os.geteuid()) if args.desktop_session else ROOT / '.local/e06-runtime'
if not args.desktop_session:
    runtime.mkdir(mode=0o700, exist_ok=True)
    runtime.chmod(0o700)
env = os.environ.copy()
env.update(XDG_RUNTIME_DIR=str(runtime), DBUS_SESSION_BUS_ADDRESS='unix:path=' + str(runtime / 'bus'))
if not args.desktop_session:
    env.update(
           XDG_STATE_HOME=str(ROOT / '.local/e06-state'), XDG_CONFIG_HOME=str(ROOT / '.local/e06-config'),
           XDG_CACHE_HOME=str(ROOT / '.local/e06-cache'))
audio = ROOT / 'target/release/neonmix-audio'
hub = ROOT / 'target/release/neonmix-hub'
children, handles = [], []
report = {'passed': False, 'scope': 'Ubuntu non-root isolated PipeWire/WirePlumber session; no desktop/physical speaker claim',
          'uid': os.geteuid(), 'binary_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in [audio, hub]}}
if args.desktop_session:
    report['scope'] = 'Ubuntu logged-in desktop PipeWire/WirePlumber session; no physical speaker claim'


def start(name, args):
    out, err = (OUT / (name + '.jsonl')).open('w'), (OUT / (name + '.stderr')).open('w')
    handles.extend([out, err])
    child = subprocess.Popen([str(arg) for arg in args], env=env, cwd=ROOT, stdout=out, stderr=err)
    children.append(child)
    return child


def checked(args, expected=0, timeout=20):
    result = subprocess.run([str(arg) for arg in args], env=env, cwd=ROOT, capture_output=True, text=True, timeout=timeout)
    assert result.returncode == expected, result.stderr
    return result.stdout


def rows(name):
    values = []
    for line in (OUT / (name + '.jsonl')).read_text().splitlines():
        try:
            values.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return values


def wait(condition, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = condition()
        if value:
            return value
        time.sleep(.1)
    raise TimeoutError('Expected native state did not arrive')


def stop(child):
    if child.poll() is None:
        child.send_signal(signal.SIGINT)
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()


try:
    if not args.desktop_session:
        dbus = start('dbus', ['dbus-daemon', '--session', '--nofork', '--address=' + env['DBUS_SESSION_BUS_ADDRESS']])
        wait(lambda: (runtime / 'bus').exists())
        pipewire = start('pipewire', ['pipewire'])
        wait(lambda: (runtime / 'pipewire-0').exists())
        manager = start('wireplumber', ['wireplumber'])
    else:
        assert (runtime / 'pipewire-0').exists(), 'Desktop PipeWire session is unavailable'
    owner_directory = ROOT / '.local/e06-owner'
    owner = start('owner', [audio, 'virtual-output', '--state-directory', owner_directory])
    wait(lambda: any(row.get('event') == 'virtual_output_ready' for row in rows('owner')))
    device = json.loads(checked([hub, 'virtual-output']))
    assert device['device']['id'] == 'pipewire:neonmix.sink.default'
    report['selected'] = device
    duplicate = subprocess.run([str(audio), 'virtual-output', '--state-directory', str(ROOT / '.local/e06-owner-second')], env=env, capture_output=True, text=True, timeout=5)
    assert duplicate.returncode != 0, 'Second owner must not create a duplicate node'
    report['duplicate_owner_rejected'] = duplicate.stderr
    bridge = subprocess.run([sys.executable, str(ROOT / 'tools/probe.py'), '--release', '--mode', 'loopback', '--device', 'pipewire:neonmix.sink.default', '--rate', '48000'], env=env, cwd=ROOT, capture_output=True, text=True, timeout=25)
    (OUT / 'bridge.log').write_text(bridge.stdout + bridge.stderr)
    shutil.copytree(ROOT / 'artifacts/virtual-probe', OUT / 'bridge')
    report['bridge'] = json.loads((OUT / 'bridge/result.json').read_text())
    assert bridge.returncode == 0 and report['bridge']['passed'], 'Actual owned sink bridge failed'
    stop(owner)
    report['owner_stopped'] = rows('owner')
    wait(lambda: not any(d['id'] == 'pipewire:neonmix.sink.default' for d in json.loads(checked([audio, 'devices']))))
    report['owner_stop_removes_node'] = True
    report['passed'] = True
except Exception as error:
    report['failure'] = repr(error)
finally:
    for child in reversed(children):
        stop(child)
    for handle in handles:
        handle.close()
    (OUT / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'failure': report.get('failure'), 'folder': str(OUT)}), flush=True)
raise SystemExit(0 if report['passed'] else 1)
