#!/usr/bin/env python3
"""macOS BlackHole: authenticated persistent binding, rename, revocation and Hub identity."""
import argparse
import hashlib
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
import traceback
from macos_audio_controls import DeviceControls

ROOT=Path(__file__).resolve().parents[1]
if sys.platform!='darwin':raise SystemExit('macOS probe only')
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--provider',choices=['blackhole','neonmix'],default='blackhole')
args=parser.parse_args()
DEVICE='coreaudio:'+('BlackHole2ch_UID' if args.provider=='blackhole' else 'com.neonmix.audio.virtual-output')
binary,audio=ROOT/'target/release/neonmix-hub',ROOT/'target/release/neonmix-audio'
out=ROOT/'artifacts/e06-binding'/time.strftime('%Y%m%d-%H%M%S');out.mkdir(parents=True)
lab=Path(tempfile.mkdtemp(prefix='e06-binding-probe-',dir=ROOT/'.local/tmp'))
directory=lab/'output';children=[];handles=[]
controls=DeviceControls(DEVICE.removeprefix('coreaudio:'));original={'rate':controls.rate(),'volume':controls.volume(),'mute':controls.muted(),'name':controls.name()}
report={'passed':False,'scope':('macOS persistent output binding and real BlackHole Sender; not HAL system loading' if args.provider=='blackhole' else 'macOS persistent output binding and installed NeonMix HAL Sender'), 'provider':args.provider,
        'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest()}

def start(name,args):
    stdout,stderr=(out/(name+'.jsonl')).open('w'),(out/(name+'.stderr')).open('w');handles.extend([stdout,stderr])
    child=subprocess.Popen([str(value) for value in args],cwd=ROOT,stdout=stdout,stderr=stderr);children.append(child);return child
def stop(child):
    if child.poll() is None:
        child.send_signal(signal.SIGINT)
        try:child.wait(timeout=8)
        except subprocess.TimeoutExpired:child.kill();child.wait()
def wait(getter,predicate=lambda value:bool(value),seconds=10):
    deadline=time.monotonic()+seconds
    while time.monotonic()<deadline:
        try:value=getter()
        except (OSError,AssertionError):value=None
        if predicate(value):return value
        time.sleep(.1)
    raise TimeoutError('expected binding state did not arrive')
def api(path):
    connection=http.client.HTTPSConnection('localhost',port,context=context,timeout=3)
    try:
        connection.request('GET',path,headers={'Authorization':'Bearer '+credential['token']});response=connection.getresponse();assert response.status==200;return json.loads(response.read())
    finally:connection.close()
def output(operation,*arguments,expected=0):
    result=subprocess.run([str(binary),'output',operation,'--directory',str(directory),*[str(v) for v in arguments]],capture_output=True,text=True,timeout=10)
    assert result.returncode==expected,result.stderr
    return json.loads(result.stdout) if result.returncode==0 else {'stderr':result.stderr}
def send(name,seconds=30):
    return start(name,[binary,'send','--credential',lab/'sender-a.json','--hub',f'https://localhost:{port}','--output-binding',directory,'--seconds',seconds])
def active():
    return next((value for value in api('/v1/hub')['sessions'].values() if value['status'] in ('buffering','playing','network_degraded')),None)
def free_port():
    with socket.socket() as free:free.bind(('127.0.0.1',0));return free.getsockname()[1]
def rows(name):return [json.loads(line) for line in (out/(name+'.jsonl')).read_text().splitlines()]

try:
    controls.set_rate(48000);controls.set_volume(1);controls.set_muted(False)
    subprocess.run([str(binary),'init','--directory',str(lab),'--output','coreaudio:BuiltInSpeakerDevice'],check=True,stdout=subprocess.DEVNULL)
    credential=json.loads((lab/'admin.json').read_text());context=ssl.create_default_context(cadata=credential['certificate']);port=free_port()
    hub=start('hub',[binary,'serve','--config',lab/'server.json','--listen',f'127.0.0.1:{port}']);wait(lambda:api('/v1/hub'))
    first=output('add','--credential',lab/'sender-a.json','--hub',f'https://localhost:{port}','--provider',args.provider,'--name','NeonMix — 客厅')
    repeated=output('add','--credential',lab/'sender-a.json','--hub',f'https://localhost:{port}','--provider',args.provider,'--name','Ignored retry name')
    assert repeated==first
    assert output('show')==first
    source=start('source',[audio,'play','--device',DEVICE,'--rate',48000,'--seconds',50,'--frequency',437,'--gain-db=-36'])
    sending=send('sender');session=wait(active)
    before=wait(lambda:api('/v1/diagnostics'),lambda d:d and len(d['receivers'])==1 and d['receivers'][0]['pcm_frames']>48000)
    renamed=output('rename','--expected-revision',first['revision'],'--name','NeonMix — 书房')
    assert renamed['output_id']==first['output_id'] and renamed['hub_id']==first['hub_id'] and renamed['device_id']==first['device_id']
    if args.provider=='neonmix':
        output('sync-name')
        wait(controls.name,lambda value:value==renamed['display_name'])
        report['native_name_sync']={'requested':renamed['display_name'],'actual':controls.name()}
    output('rename','--expected-revision',first['revision'],'--name','Conflict',expected=1)
    time.sleep(1)
    after=api('/v1/diagnostics');assert sending.poll() is None
    assert after['receivers'][0]['pcm_frames']>before['receivers'][0]['pcm_frames'] and after['receivers'][0]['last_buffer_rms']>.008
    report['rename_preserves_stream']={'before':before,'after':after,'original':first,'renamed':renamed}
    disabled=output('disable','--expected-revision',renamed['revision'])
    enabled=output('enable','--expected-revision',disabled['revision'])
    assert sending.wait(timeout=8)!=0
    assert any(row['event']=='output_binding_revoked' for row in rows('sender'))
    time.sleep(.5);assert not active()
    report['disable_enable_stops_old_sender']={'disabled':disabled,'enabled':enabled,'closed_session':api('/v1/hub')['sessions'][session['id']]}
    restarted=send('sender-restarted');fresh=wait(active);assert fresh['id']!=session['id'] and fresh['media_context']!=session['media_context']
    wait(lambda:api('/v1/diagnostics'),lambda d:d and len(d['receivers'])==1 and d['receivers'][0]['pcm_frames']>48000)
    output('remove','--expected-revision',enabled['revision'])
    assert restarted.wait(timeout=8)!=0;assert not active()
    replacement=output('add','--credential',lab/'sender-a.json','--hub',f'https://localhost:{port}','--provider',args.provider,'--name','NeonMix — 新绑定')
    assert replacement['output_id']!=first['output_id']
    report['explicit_replacement']=replacement
    # Move the same authoritative Hub to another address; the output identity remains stable.
    stop(hub);port=free_port()
    hub=start('hub-new-address',[binary,'serve','--config',lab/'server.json','--listen',f'127.0.0.1:{port}']);wait(lambda:api('/v1/hub'))
    relocated=send('sender-new-address',4)
    state=wait(active);assert state['id']!=fresh['id']
    assert relocated.wait(timeout=8)==0
    assert output('show')==replacement
    report['address_change_keeps_binding']={'hub_id':api('/v1/hub')['hub_id'],'binding':output('show')}
    # A record that becomes readable/writable to other users also revokes the live path.
    permissions=send('sender-private-file');wait(active)
    wait(lambda:api('/v1/diagnostics'),lambda d:d and len(d['receivers'])==1 and d['receivers'][0]['pcm_frames']>48000)
    (directory/'binding.json').chmod(0o644)
    assert permissions.wait(timeout=8)!=0
    (directory/'binding.json').chmod(0o600)
    assert not active() and output('show')==replacement
    report['private_file_permission_loss_stops_sender']=rows('sender-private-file')
    stop(hub)
    # Same TLS certificate and member tokens, different authority UUID: TLS alone is insufficient.
    config=json.loads((lab/'server.json').read_text());config['state_path']=str(lab/'other-state.json')
    other=lab/'other-server.json';other.write_text(json.dumps(config));other.chmod(0o600)
    port=free_port();hub=start('other-hub',[binary,'serve','--config',other,'--listen',f'127.0.0.1:{port}']);wait(lambda:api('/v1/hub'))
    assert api('/v1/hub')['hub_id']!=replacement['hub_id']
    rejected=send('sender-wrong-hub');assert rejected.wait(timeout=8)!=0
    diagnostic=api('/v1/diagnostics');assert not diagnostic['receivers']
    report['different_hub_rejected_before_media']=diagnostic
    report['passed']=True
except Exception as error:
    report['error']=repr(error);report['traceback']=traceback.format_exc()
finally:
    for child in reversed(children):stop(child)
    for handle in handles:handle.close()
    controls.set_rate(original['rate']);controls.set_volume(original['volume']);controls.set_muted(original['mute'])
    if args.provider=='neonmix' and controls.name()!=original['name']:controls.set_owned_name(original['name'])
    shutil.rmtree(lab)
    (out/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'passed':report['passed'],'error':report.get('error'),'evidence':str(out)}),flush=True)
raise SystemExit(0 if report['passed'] else 1)
