#!/usr/bin/env python3
"""macOS E07 real IPC + virtual capture + second synthetic Sender controls.
Loopback checks function and process boundaries, not a two-host LAN acceptance.
"""
import hashlib
import json
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import time
import traceback
import uuid
from e07_background_probe import ipc, wait_for
from credential_fixture import remove_owned_fixture

ROOT=Path(__file__).resolve().parents[1]
DEV=ROOT/'tools/dev'
HUB=ROOT/'target/release/neonmix-hub'
AUDIO=ROOT/'target/release/neonmix-audio'
BACKGROUND=ROOT/'target/release/neonmix-background'
URL='https://localhost:7443'

def main():
    assert platform.system()=='Darwin','macOS-only probe'
    base=ROOT/'.local/tmp'/f'e07-mixer-{uuid.uuid4().hex[:12]}'
    base.mkdir(mode=0o700)
    hub_dir=base/'hub-owner';sender_dir=base/'sender-owner'
    hub_dir.mkdir(mode=0o700);sender_dir.mkdir(mode=0o700)
    report={'platform':'macOS','scope':'loopback with independent Rust background processes, virtual capture A and synthetic source B','passed':False,'scenarios':{},'binary_sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [HUB,AUDIO,BACKGROUND]}}
    processes=[];handles=[]
    def spawn(binary,*args):
        log=(base/f'process-{len(processes)}.log').open('w');handles.append(log)
        child=subprocess.Popen([str(DEV),str(binary),*map(str,args)],stdout=log,stderr=log,cwd=ROOT)
        processes.append(child);return child
    def cli(*args):
        result=subprocess.run([str(DEV),str(HUB),*map(str,args)],capture_output=True,cwd=ROOT,timeout=30)
        assert result.returncode==0,result.stderr.decode()[:300]
        return json.loads(result.stdout.splitlines()[-1])
    def snapshot():return ipc(hub_dir,'snapshot',credential='hub/admin.json',hub=URL)
    def diagnostics():return ipc(hub_dir,'diagnostics',credential='hub/admin.json',hub=URL)['remote']
    def control(operation,credential='hub/admin.json',revision=None):
        state=snapshot()
        return ipc(hub_dir,'control',credential=credential,hub=URL,expected_revision=state['revision'] if revision is None else revision,operation=operation)
    def lane(d,id):return next(v for v in d['meters']['lanes'] if v['stream_id']==id)
    try:
        spawn(BACKGROUND,'--state-dir',hub_dir);spawn(BACKGROUND,'--state-dir',sender_dir)
        wait_for(lambda:ipc(hub_dir,'status'));wait_for(lambda:ipc(sender_dir,'status'))
        devices=ipc(hub_dir,'devices')
        assert any(d['id']=='coreaudio:BuiltInSpeakerDevice' and d['output'] for d in devices)
        assert any(d['id']=='coreaudio:BlackHole2ch_UID' and d['input'] and d['output'] for d in devices)
        ipc(hub_dir,'hub_setup',settings={'name':'E07 双路测试','output':'coreaudio:BuiltInSpeakerDevice'})
        ipc(hub_dir,'hub_start');wait_for(snapshot)
        invite=ipc(hub_dir,'invite',credential='hub/admin.json',hub=URL,out='invitations/a.json',seconds=120)
        paired=ipc(sender_dir,'pair_text',invitation=invite['invitation'],name='E07 真实采集 A',hub=URL)
        metadata=json.loads((sender_dir/'profiles/sender.json').read_text())
        assert metadata['version']==2 and metadata['credential_store']=='file' and metadata['profile_kind']=='member'
        member_id=paired['device_id']
        state=ipc(sender_dir,'snapshot',credential='profiles/sender.json',hub=URL)
        assert state['viewer']['role']=='member'
        binding=ipc(sender_dir,'output',directory='output',action={'action':'add','credential':'profiles/sender.json','hub':URL,'name':'NeonMix — E07 双路测试','provider':'blackhole','device':None})
        assert binding['enabled'] and binding['hub_id']==snapshot()['hub_id']
        restored=ipc(sender_dir,'status')['output_binding'];assert restored['output_id']==binding['output_id']
        renamed=ipc(sender_dir,'output',directory='output',action={'action':'rename','expected_revision':binding['revision'],'expected_output_id':binding['output_id'],'name':'NeonMix — E07 采集'})
        assert renamed['output_id']==binding['output_id']
        report['scenarios']['pair_text_member_role_binding_and_identity_preserving_rename']=True
        invite_b=ipc(hub_dir,'invite',credential='hub/admin.json',hub=URL,out='invitations/b.json',seconds=120)
        invite_path=hub_dir/'invitations/b-copy.json';invite_path.write_text(invite_b['invitation']);invite_path.chmod(0o600)
        (hub_dir/'profiles').mkdir(exist_ok=True,mode=0o700)
        paired_b=cli('pair','--invite',invite_path,'--credential',hub_dir/'profiles/b.json','--name','E07 合成 B','--hub',URL)
        ipc(sender_dir,'sender_start',options={'credential':'profiles/sender.json','hub':URL,'output_binding':'output'})
        spawn(HUB,'send','--credential',hub_dir/'profiles/b.json','--hub',URL,'--seconds',90,'--frequency',659)
        source=spawn(AUDIO,'play','--device','coreaudio:BlackHole2ch_UID','--seconds',90,'--frequency',437,'--gain-db',-36)
        state=wait_for(lambda:(s if len(s['streams'])==2 else None) if (s:=snapshot()) else None)
        a=next(v['id'] for v in state['streams'].values() if v['device_id']==member_id)
        b=next(v['id'] for v in state['streams'].values() if v['device_id']==paired_b['device_id'])
        d=wait_for(lambda:(d if lane(d,a)['rms']>0.002 and lane(d,b)['rms']>0.002 and d['meters']['output']['rms']>0 else None) if (d:=diagnostics()) else None,seconds=15)
        assert ipc(sender_dir,'status')['sender']['running']
        capture=wait_for(lambda:(m if m and m.get('capture_stats',{}).get('frames',0)>0 else None) if (m:=ipc(sender_dir,'status')['sender'].get('metrics')) else None)
        report['scenarios']['actual_virtual_capture_and_two_authenticated_stream_meters']=True
        report['meter_baseline']={'a':lane(d,a),'b':lane(d,b),'output':d['meters']['output']}
        fixture={'devices':devices,'status':ipc(hub_dir,'status'),'snapshot':state,'diagnostics':d}
        fixture_path=ROOT/'artifacts/e07/ui-real-state.json';fixture_path.parent.mkdir(parents=True,exist_ok=True);fixture_path.write_text(json.dumps(fixture,ensure_ascii=False))
        baseline_b=lane(d,b)['rms']
        ipc(sender_dir,'control',credential='profiles/sender.json',hub=URL,expected_revision=snapshot()['revision'],operation={'type':'stream_mix','stream_id':a,'gain_db':None,'muted':True,'solo':None})
        wait_for(lambda:lane(diagnostics(),a)['rms']<1e-6)
        assert lane(diagnostics(),b)['rms']>baseline_b*0.5
        control({'type':'stream_mix','stream_id':a,'gain_db':None,'muted':False,'solo':None})
        wait_for(lambda:lane(diagnostics(),a)['rms']>0.002)
        report['scenarios']['member_own_mute_and_independent_other_stream']=True
        try:
            ipc(sender_dir,'control',credential='profiles/sender.json',hub=URL,expected_revision=snapshot()['revision'],operation={'type':'stream_mix','stream_id':b,'gain_db':None,'muted':True,'solo':None})
            raise AssertionError('member controlled another stream')
        except RuntimeError as error:assert 'permission_denied' in str(error)
        report['scenarios']['server_denies_member_other_stream_control']=True
        control({'type':'stream_mix','stream_id':a,'gain_db':None,'muted':None,'solo':True})
        wait_for(lambda:lane(diagnostics(),b)['rms']<1e-6)
        assert lane(diagnostics(),a)['rms']>0.002
        control({'type':'stream_mix','stream_id':b,'gain_db':None,'muted':None,'solo':True})
        wait_for(lambda:lane(diagnostics(),b)['rms']>0.002)
        for stream in (a,b):control({'type':'stream_mix','stream_id':stream,'gain_db':None,'muted':None,'solo':False})
        report['scenarios']['admin_solo_and_multiple_solo_meters']=True
        before=diagnostics()['meters']['output']['rms']
        control({'type':'output_mix','gain_db':-24,'muted':None})
        wait_for(lambda:diagnostics()['meters']['output']['rms']<before*0.4)
        control({'type':'output_mix','gain_db':None,'muted':True})
        wait_for(lambda:diagnostics()['meters']['output']['rms']<1e-6)
        control({'type':'output_mix','gain_db':-12,'muted':False})
        report['scenarios']['room_gain_mute_and_real_post_limiter_meter']=True
        old=snapshot()['revision']
        control({'type':'stream_mix','stream_id':a,'gain_db':-12,'muted':None,'solo':None})
        wait_for(lambda:lane(diagnostics(),a)['rms']<0.004)
        try:
            control({'type':'output_mix','gain_db':None,'muted':True},revision=old)
            raise AssertionError('stale revision accepted')
        except RuntimeError as error:assert 'revision_conflict' in str(error)
        assert not snapshot()['output']['muted']
        report['scenarios']['gain_and_stale_revision_conflict_without_overwrite']=True
        exported=ipc(hub_dir,'export_diagnostics',credential='hub/admin.json',hub=URL)
        export=(hub_dir/'diagnostics-redacted.json').read_text()
        for secret in [invite['invitation'],invite_b['invitation'],str(base),'E07 真实采集','coreaudio:','BEGIN CERTIFICATE','secret_ref']:
            assert secret not in export
        assert 'meters' in export and exported['file']=='diagnostics-redacted.json'
        report['scenarios']['numeric_whitelist_diagnostic_export']=True
        control({'type':'disconnect','device_id':member_id})
        wait_for(lambda:not ipc(sender_dir,'status')['sender']['running'])
        assert lane(diagnostics(),b)['rms']>baseline_b*0.5
        control({'type':'allow_playback','device_id':member_id})
        time.sleep(0.3);assert not ipc(sender_dir,'status')['sender']['running']
        report['scenarios']['admin_disconnect_and_allow_does_not_autostart']=True
        ipc(sender_dir,'sender_start',options={'credential':'profiles/sender.json','hub':URL,'output_binding':'output'})
        wait_for(lambda:any(v['device_id']==member_id for v in snapshot()['streams'].values()))
        ipc(sender_dir,'sender_stop');assert not ipc(sender_dir,'status')['sender']['running']
        assert ipc(hub_dir,'status')['hub']['running']
        report['scenarios']['explicit_sender_restart_and_stop_preserve_hub']=True
        current=ipc(sender_dir,'output',directory='output',action={'action':'show'})
        disabled=ipc(sender_dir,'output',directory='output',action={'action':'disable','expected_revision':current['revision'],'expected_output_id':current['output_id']})
        enabled=ipc(sender_dir,'output',directory='output',action={'action':'enable','expected_revision':disabled['revision'],'expected_output_id':disabled['output_id']})
        ipc(sender_dir,'output',directory='output',action={'action':'remove','expected_revision':enabled['revision'],'expected_output_id':enabled['output_id']})
        assert ipc(sender_dir,'status')['output_binding'] is None
        report['scenarios']['binding_disable_enable_remove_and_reopen_status']=True
        control({'type':'revoke','device_id':paired_b['device_id']})
        wait_for(lambda:snapshot()['devices'][paired_b['device_id']]['revoked'])
        report['scenarios']['revoke_closes_active_sender']=True
        report['passed']=True
    except Exception as error:
        report['failure']=str(error);report['traceback']=traceback.format_exc();raise
    finally:
        for directory in [sender_dir,hub_dir]:
            try:ipc(directory,'shutdown')
            except (OSError,RuntimeError):pass
        for child in reversed(processes):
            if child.poll() is None:
                child.terminate()
                try:child.wait(timeout=8)
                except subprocess.TimeoutExpired:child.kill();child.wait()
        for handle in handles:handle.close()
        remove_owned_fixture(base)
        report['fixture_files_removed']=not base.exists()
        path=ROOT/'docs/evidence/e07/mixer-macos.json';path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n')
    print(json.dumps(report,ensure_ascii=False))

if __name__=='__main__':main()
