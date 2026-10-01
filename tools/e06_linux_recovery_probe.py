#!/usr/bin/env python3
"""Explicit desktop PipeWire/WirePlumber restart probe; run only on a test host."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[1]
if sys.platform != 'linux' or os.geteuid() == 0:
    raise SystemExit('Requires the logged-in non-root PipeWire user')
OUT = ROOT / 'artifacts/e06-linux-recovery' / time.strftime('%Y%m%d-%H%M%S')
OUT.mkdir(parents=True)
audio = ROOT / 'target/release/neonmix-audio'
children, handles = [], []
report = {'passed': False, 'uid': os.geteuid(), 'scope': 'Actual Ubuntu desktop service restarts; no logout/sleep/physical audio claim',
          'binary_sha256': hashlib.sha256(audio.read_bytes()).hexdigest()}


def checked(args, timeout=20):
    result = subprocess.run([str(v) for v in args], cwd=ROOT, capture_output=True, text=True, timeout=timeout)
    assert result.returncode == 0, result.stdout + result.stderr
    return result.stdout


def wait(getter, predicate=bool, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try: value = getter()
        except (OSError, AssertionError, json.JSONDecodeError): value = None
        if predicate(value): return value
        time.sleep(.1)
    raise TimeoutError('Native recovery did not arrive')


def node():
    nodes = [n for n in json.loads(checked(['pw-dump'])) if n.get('info', {}).get('props', {}).get('node.name') == 'neonmix.sink.default']
    assert len(nodes) == 1, 'Exactly one owned node is required'
    return nodes[0]


def pid(service):
    return int(checked(['systemctl','--user','show',service,'--property=MainPID','--value']).strip())


def start(name, args):
    out, err = (OUT/(name+'.jsonl')).open('w'), (OUT/(name+'.stderr')).open('w')
    handles.extend([out,err]); child = subprocess.Popen([str(v) for v in args], cwd=ROOT, stdout=out, stderr=err)
    children.append(child); return child


def rows(name):
    return [json.loads(line) for line in (OUT/(name+'.jsonl')).read_text().splitlines() if line.startswith('{')]


def bridge(name):
    result = subprocess.run([sys.executable,str(ROOT/'tools/probe.py'),'--release','--mode','loopback','--device','pipewire:neonmix.sink.default','--rate','48000'],cwd=ROOT,capture_output=True,text=True,timeout=25)
    (OUT/(name+'.log')).write_text(result.stdout + result.stderr)
    shutil.copytree(ROOT/'artifacts/virtual-probe',OUT/name)
    summary = json.loads((OUT/name/'result.json').read_text()); assert result.returncode == 0 and summary['passed']; return summary


try:
    owner_pid = pid('neonmix-e06-owner'); assert owner_pid > 0
    original = wait(node); report['original_node'] = original
    old_manager = pid('wireplumber')
    checked(['systemctl','--user','restart','wireplumber'])
    wait(lambda:pid('wireplumber'),lambda p:p and p != old_manager)
    time.sleep(2)
    same = wait(node); assert same['info']['props']['node.name'] == original['info']['props']['node.name']
    assert same['info']['props']['node.description'] == original['info']['props']['node.description']
    report['wireplumber'] = {'before_pid':old_manager,'after_pid':pid('wireplumber'),'recovered_node':same,'bridge':bridge('after-manager')}
    capture = start('old-capture',[audio,'capture','--device','pipewire:neonmix.sink.default','--mode','loopback','--rate',48000,'--seconds',60])
    output = start('old-output',[audio,'play','--device','pipewire:neonmix.sink.default','--signal','silence','--rate',48000,'--seconds',60])
    wait(lambda:rows('old-capture'),lambda r:r and any(v['event']=='capture_stats' for v in r))
    wait(lambda:rows('old-output'),lambda r:r and any(v['event']=='output_stats' for v in r))
    old_pipewire = pid('pipewire')
    checked(['systemctl','--user','restart','pipewire'])
    codes = {'capture':capture.wait(timeout=8),'output':output.wait(timeout=8)}
    assert all(code != 0 for code in codes.values()), 'Existing streams must fail explicitly'
    recovered = wait(node)
    assert pid('pipewire') != old_pipewire and pid('neonmix-e06-owner') == owner_pid
    assert recovered['info']['props']['node.description'] == original['info']['props']['node.description']
    report['pipewire'] = {'before_pid':old_pipewire,'after_pid':pid('pipewire'),'owner_pid':owner_pid,
                          'old_stream_exit_codes':codes,'recovered_node':recovered,'bridge':bridge('after-pipewire')}
    removed_serial = recovered['info']['props']['object.serial']
    checked(['pw-cli','destroy',recovered['id']])
    replaced = wait(node,lambda n:n and n['info']['props']['object.serial'] != removed_serial)
    assert pid('neonmix-e06-owner') == owner_pid
    report['node_removal'] = {'old_serial':removed_serial,'new_node':replaced,'bridge':bridge('after-removal')}
    report['passed'] = True
except Exception as error:
    report['failure'] = repr(error)
    report['traceback'] = traceback.format_exc()
finally:
    for child in reversed(children):
        if child.poll() is None:
            child.send_signal(signal.SIGINT)
            try:child.wait(timeout=5)
            except subprocess.TimeoutExpired:child.kill();child.wait()
    for handle in handles:handle.close()
    (OUT/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'passed':report['passed'],'failure':report.get('failure'),'folder':str(OUT)}),flush=True)
raise SystemExit(0 if report['passed'] else 1)
