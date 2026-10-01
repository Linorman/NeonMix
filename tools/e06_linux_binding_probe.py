#!/usr/bin/env python3
"""Real desktop owner, application selection, endpoint gain and binding revocation."""
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import wave
import struct

ROOT = Path(__file__).resolve().parents[1]
if sys.platform != 'linux' or os.geteuid() == 0:
    raise SystemExit('Requires the logged-in non-root PipeWire user')
audio, binary = ROOT / 'target/release/neonmix-audio', ROOT / 'target/release/neonmix-hub'
DEVICE = 'pipewire:neonmix.sink.default'
OUT = ROOT / 'artifacts/e06-linux-binding' / time.strftime('%Y%m%d-%H%M%S')
OUT.mkdir(parents=True)
lab = Path(tempfile.mkdtemp(prefix='e06-binding-', dir=ROOT / '.local/tmp'))
directory = ROOT / '.local/e06-binding'
assert not (directory / 'binding.json').exists(), 'Do not overwrite an existing user binding'
children, handles = [], []
report = {'passed': False, 'uid': os.geteuid(), 'scope': 'Ubuntu actual desktop owner, pw-play, volume and encrypted Sender binding',
          'binary_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in [audio, binary]}}


def start(name, args):
    out, err = (OUT / (name + '.jsonl')).open('w'), (OUT / (name + '.stderr')).open('w')
    handles.extend([out, err]); child = subprocess.Popen([str(v) for v in args], cwd=ROOT, stdout=out, stderr=err)
    children.append(child); return child


def stop(child):
    if child.poll() is None:
        child.send_signal(signal.SIGINT)
        try: child.wait(timeout=5)
        except subprocess.TimeoutExpired: child.kill(); child.wait()


def checked(args, expected=0):
    r = subprocess.run([str(v) for v in args], cwd=ROOT, capture_output=True, text=True, timeout=15)
    assert r.returncode == expected, r.stderr
    return r.stdout


def wait(getter, predicate=bool, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try: value = getter()
        except (OSError, AssertionError): value = None
        if predicate(value): return value
        time.sleep(.1)
    raise TimeoutError('Expected native binding state did not arrive')


def node():
    nodes = [n for n in json.loads(checked(['pw-dump'])) if n.get('info', {}).get('props', {}).get('node.name') == 'neonmix.sink.default']
    assert len(nodes) == 1, 'Owned sink must be unique'
    return nodes[0]


def rows(name):
    return [json.loads(line) for line in (OUT / (name + '.jsonl')).read_text().splitlines() if line.startswith('{')]


def measurement():
    return next((row for row in reversed(rows('capture')) if row.get('event') == 'capture_stats'), None)


def phase(name):
    # Stats are cumulative and emitted once a second. Skip two snapshots after
    # the change so the measured interval cannot include the preceding gain.
    previous = wait(measurement)
    for _ in range(2):
        previous = wait(measurement, lambda r: r and r['stats']['frames'] > previous['stats']['frames'])
    before = previous; time.sleep(1.5)
    after = wait(measurement, lambda r: r and r['stats']['frames'] - before['stats']['frames'] >= 48000)
    a,b = before['measurement'],after['measurement']; frames=b['analyzed_frames']-a['analyzed_frames']
    assert all(after['stats'][k] == 0 for k in ('errors','callback_over_budget','dropped_frames','stale_frames')), after['stats']
    rms=math.sqrt(max(0,(b['rms']**2*b['analyzed_frames']-a['rms']**2*a['analyzed_frames'])/frames))
    value={'rms':rms,'before':before,'after':after,'native_props':node()['info']['params']['Props']};report.setdefault('phases',{})[name]=value;return value


def api(path):
    connection=http.client.HTTPSConnection('localhost',port,context=context,timeout=3)
    try:
        connection.request('GET',path,headers={'Authorization':'Bearer '+credential['token']});r=connection.getresponse();assert r.status==200;return json.loads(r.read())
    finally: connection.close()


def output(operation,*args):
    return json.loads(checked([binary,'output',operation,'--directory',directory,*args]))


def active():
    return next((s for s in api('/v1/hub')['sessions'].values() if s['status'] in ('buffering','playing','network_degraded')),None)


try:
    original=node(); node_id=original['id'];report['original_node']=original
    target=start('target',[audio,'sink','--room','e06-hub','--seconds',180])
    wait(lambda:any(d['id']=='pipewire:neonmix.sink.e06-hub' for d in json.loads(checked([audio,'devices']))))
    checked([binary,'init','--directory',lab,'--output','pipewire:neonmix.sink.e06-hub'])
    credential=json.loads((lab/'admin.json').read_text());context=ssl.create_default_context(cadata=credential['certificate'])
    with socket.socket() as free:free.bind(('127.0.0.1',0));port=free.getsockname()[1]
    hub=start('hub',[binary,'serve','--config',lab/'server.json','--listen',f'127.0.0.1:{port}']);wait(lambda:api('/v1/hub'))
    first=output('add','--credential',lab/'sender-a.json','--hub',f'https://localhost:{port}','--name','NeonMix — Ubuntu room')
    wait(node,lambda n:n and n['info']['props'].get('node.description')==first['display_name'])
    assert node()['id']==node_id
    output('sync-name')
    tone=lab/'tone.wav'; second_tone=lab/'second-tone.wav'
    for path,frequency in ((tone,437),(second_tone,659)):
        with wave.open(str(path),'wb') as wav:
            wav.setnchannels(2);wav.setsampwidth(2);wav.setframerate(44100)
            second=b''.join(struct.pack('<hh',*([round(32767*10**(-36/20)*math.sin(2*math.pi*frequency*i/44100))]*2)) for i in range(44100))
            for _ in range(150):wav.writeframesraw(second)
    checked(['wpctl','set-volume',node_id,1]);checked(['wpctl','set-mute',node_id,0])
    capture=start('capture',[audio,'capture','--device',DEVICE,'--mode','loopback','--rate',48000,'--seconds',150])
    source=start('pw-play',['pw-play','--target','neonmix.sink.default',tone]);wait(measurement)
    baseline=phase('full_volume')['rms'];assert .01<baseline<.013
    # Set the native linear amplitude directly; wpctl uses a cubic UI scale.
    checked(['pw-cli','set-param',node_id,'Props','{ channelVolumes: [ 0.5, 0.5 ] }'])
    half=phase('half_volume')['rms'];assert .48<half/baseline<.52, 'Native volume must apply once'
    checked(['wpctl','set-mute',node_id,1]);time.sleep(.2);assert phase('mute')['rms']<1e-6
    checked(['wpctl','set-volume',node_id,1]);checked(['wpctl','set-mute',node_id,0]);time.sleep(.2)
    assert .95<phase('restored')['rms']/baseline<1.05
    second_source=start('pw-play-second',['pw-play','--target','neonmix.sink.default',second_tone])
    assert 1.38<phase('two_applications')['rms']/baseline<1.45
    stop(second_source)
    sender=start('sender',[binary,'send','--credential',lab/'sender-a.json','--hub',f'https://localhost:{port}','--output-binding',directory,'--seconds',90])
    session=wait(active);wait(lambda:api('/v1/diagnostics'),lambda d:d and len(d['receivers'])==1 and d['receivers'][0]['last_buffer_rms']>.008)
    renamed=output('rename','--expected-revision',first['revision'],'--name','NeonMix — Ubuntu renamed')
    output('sync-name');wait(node,lambda n:n and n['info']['props'].get('node.description')==renamed['display_name'])
    assert node()['id']==node_id and sender.poll() is None
    report['rename_keeps_node_and_media']={'node':node(),'binding':renamed,'diagnostics':api('/v1/diagnostics')}
    disabled=output('disable','--expected-revision',renamed['revision']);enabled=output('enable','--expected-revision',disabled['revision'])
    assert sender.wait(timeout=8)!=0 and not active();assert node()['id']==node_id
    report['disable_enable_stops_old_sender']={'old_session':api('/v1/hub')['sessions'][session['id']],'enabled':enabled}
    fresh_sender=start('sender-new',[binary,'send','--credential',lab/'sender-a.json','--hub',f'https://localhost:{port}','--output-binding',directory,'--seconds',60])
    fresh=wait(active);assert fresh['id']!=session['id'] and fresh['media_context']!=session['media_context']
    output('remove','--expected-revision',enabled['revision']);assert fresh_sender.wait(timeout=8)!=0 and not active()
    wait(node,lambda n:n and n['info']['props'].get('node.description')==original['info']['props']['node.description'])
    assert node()['id']==node_id;report['remove_preserves_local_output']=True
    stop(source)
    assert phase('no_source')['rms']<1e-6
    report['passed']=True
except Exception as error:
    report['failure']=repr(error)
finally:
    for child in reversed(children):stop(child)
    for handle in handles:handle.close()
    try:
        current=node();props=original['info']['params']['Props'][0]
        checked(['pw-cli','set-param',current['id'],'Props',json.dumps({k:props[k] for k in ('volume','mute','channelVolumes','softMute','softVolumes','monitorMute','monitorVolumes')})])
        if (directory/'binding.json').exists():
            binding=output('show');output('remove','--expected-revision',binding['revision'])
    except Exception as error:report['restore_failure']=repr(error);report['passed']=False
    shutil.rmtree(lab)
    (OUT/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'passed':report['passed'],'failure':report.get('failure'),'folder':str(OUT)}),flush=True)
raise SystemExit(0 if report['passed'] else 1)
