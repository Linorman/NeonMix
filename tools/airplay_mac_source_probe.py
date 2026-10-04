#!/usr/bin/env python3
"""Use the actual macOS Sound output selector against an isolated local Speaker.
Restores the original system output and cleans fixture keys/files. Does not claim
physical A/V synchronization; successful audio is a separate digital smoke check.
"""
import argparse, json, math, os, shutil, ssl, subprocess, sys, time, uuid, wave, struct, re, hashlib, ctypes as C
import plistlib
from pathlib import Path
from urllib.request import Request, urlopen
from airplay_hub_probe import airplay_session, session_meter, wait_for
from macos_audio_controls import DeviceControls
from credential_fixture import remove_owned_fixture

ROOT=Path(__file__).resolve().parents[1]
DEV=ROOT/'tools/dev'

def ax(body):
    result=subprocess.run([str(DEV),'/usr/bin/osascript','-'],input=body,capture_output=True,text=True,timeout=12,cwd=ROOT)
    if result.returncode:
        code=re.search(r'\((-?\d+)\)\s*$',result.stderr)
        raise RuntimeError('AX '+(code.group(1) if code else 'failure'))
    return result.stdout.strip()

def mouse_click(x,y):
    class Point(C.Structure):_fields_=[('x',C.c_double),('y',C.c_double)]
    cg=C.CDLL('/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics')
    cf=C.CDLL('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation')
    cg.CGEventCreateMouseEvent.argtypes=[C.c_void_p,C.c_uint,Point,C.c_uint];cg.CGEventCreateMouseEvent.restype=C.c_void_p
    cg.CGEventPost.argtypes=[C.c_uint,C.c_void_p];cf.CFRelease.argtypes=[C.c_void_p]
    for kind in [5,1,2]:
        event=cg.CGEventCreateMouseEvent(None,kind,Point(x,y),0);cg.CGEventPost(0,event);cf.CFRelease(event)
        time.sleep(.05)

def rows():
    try:return ax('tell application "System Events" to tell process "System Settings" to return value of every static text of group 1 of UI element 1 of every row of outline 1 of scroll area 1 of group 2 of scroll area 1 of group 1 of group 3 of splitter group 1 of group 1 of window 1')
    except RuntimeError:return ''

def pairing_ui_snapshot():
    """Read only the source pairing agent's own windows; retain no desktop pixels."""
    cf=C.CDLL('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation')
    cg=C.CDLL('/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics')
    cg.CGWindowListCopyWindowInfo.argtypes=[C.c_uint,C.c_uint];cg.CGWindowListCopyWindowInfo.restype=C.c_void_p
    cf.CFPropertyListCreateData.argtypes=[C.c_void_p,C.c_void_p,C.c_long,C.c_ulong,C.c_void_p];cf.CFPropertyListCreateData.restype=C.c_void_p
    cf.CFDataGetBytePtr.argtypes=[C.c_void_p];cf.CFDataGetBytePtr.restype=C.POINTER(C.c_ubyte)
    cf.CFDataGetLength.argtypes=[C.c_void_p];cf.CFDataGetLength.restype=C.c_long
    cf.CFRelease.argtypes=[C.c_void_p]
    ref=cg.CGWindowListCopyWindowInfo(0,0);data=None
    try:
        data=cf.CFPropertyListCreateData(None,ref,100,0,None)
        windows=plistlib.loads(C.string_at(cf.CFDataGetBytePtr(data),cf.CFDataGetLength(data)))
        result={'cg_windows':[{'onscreen':w.get('kCGWindowIsOnscreen',False),'bounds':w['kCGWindowBounds']} for w in windows if w.get('kCGWindowOwnerName')=='AirPlayUIAgent']}
        try:result['ax_state']=ax('tell application "System Events" to tell process "AirPlayUIAgent" to return {visible, background only, count of windows}')
        except RuntimeError as error:result['ax_state']=str(error)
        return result
    finally:
        if data:cf.CFRelease(data)
        cf.CFRelease(ref)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile',choices=['debug','release'],default=os.environ.get('NEONMIX_SOURCE_BUILD_PROFILE','debug'))
    parser.add_argument('--output',default='coreaudio:BlackHole2ch_UID')
    args=parser.parse_args()
    assert sys.platform=='darwin'
    base=ROOT/'.local/tmp'/('airplay-mac-'+uuid.uuid4().hex[:12]);base.mkdir(mode=0o700,parents=True)
    profile=base/'profile';profile.mkdir(mode=0o700)
    build_profile=args.profile
    if build_profile not in ('debug','release'):raise ValueError('Invalid source probe build profile')
    binary=ROOT/'target'/build_profile/'neonmix-hub'
    hub=None;log=None;source=None;capture=None;agent_visibility=None;target=None
    controls=DeviceControls('BuiltInSpeakerDevice');original=controls.default_outputs()
    report={'platform':'macOS','build_profile':build_profile,'output_device':args.output,
        'binaries':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [binary,binary.with_name('neonmix-airplay-worker')]},
        'os_build':subprocess.check_output([str(DEV),'sw_vers','-buildVersion'],text=True).strip(),
        'source':'actual macOS system Sound output selector, same-host receiver',
        'discovered':False,'connected':False,'audio_played':False,'video_av_sync':'not measured','passed':False}
    try:
        controls.select_default_output()
        subprocess.run([str(DEV),str(binary),'init','--directory',str(profile),'--output',args.output],check=True,stdout=subprocess.DEVNULL,cwd=ROOT)
        config=json.loads((profile/'server.json').read_text());config['room_name']='AirPlay Mac '+base.name[-4:];(profile/'server.json').write_text(json.dumps(config))
        credential=json.loads((profile/'admin.json').read_text());tls=ssl.create_default_context(cadata=credential['certificate'])
        log=(base/'hub.log').open('w');hub=subprocess.Popen([str(DEV),str(binary),'serve','--config',str(profile/'server.json'),'--listen','0.0.0.0:0'],stdout=log,stderr=log,cwd=ROOT,env={**os.environ,'NEONMIX_AIRPLAY_TRACE':'1'})
        def started():
            for line in (base/'hub.log').read_text().splitlines():
                try:event=json.loads(line)
                except ValueError:continue
                if event.get('event')=='hub_started':return event['listen'].rsplit(':',1)[1]
        port=wait_for('fixture Hub',started)
        def api(path='/v1/airplay',operation=None):
            payload=None if operation is None else json.dumps(operation).encode()
            request=Request('https://localhost:'+port+path,data=payload,headers={'Authorization':'Bearer '+credential['token'],'Content-Type':'application/json'})
            with urlopen(request,context=tls,timeout=4) as response:return json.load(response)
        current=api();api(operation={'expected_revision':current['revision'],'operation':{'action':'enable'}})
        ready=wait_for('fixture Speaker ready',lambda:(s if s['ready'] else None) if (s:=api()) else None,25)
        print('Speaker ready; inspecting native macOS Sound selector.',flush=True)
        subprocess.run([str(DEV),'open','x-apple.systempreferences:com.apple.Sound-Settings.extension'],check=True,cwd=ROOT)
        target='NeonMix — '+config['room_name']
        found=wait_for('native Sound AirPlay target',lambda:target in rows(),20)
        report['discovered']=bool(found)
        print('Native Sound selector contains the Speaker; selecting it.',flush=True)
        report['pairing_ui_before']=pairing_ui_snapshot()
        if os.environ.get('NEONMIX_SOURCE_SHOW_AGENT')=='1':
            agent_visibility=ax('tell application "System Events" to tell process "AirPlayUIAgent" to return visible')
            ax('tell application "System Events" to tell process "AirPlayUIAgent" to set visible to true')
        select='''tell application "System Events" to tell process "System Settings"
set frontmost to true
set outputRows to rows of outline 1 of scroll area 1 of group 2 of scroll area 1 of group 1 of group 3 of splitter group 1 of group 1 of window 1
repeat with r in outputRows
try
if value of static text 1 of group 1 of UI element 1 of r is "TARGET" then
select r
perform action "AXShowDefaultUI" of r
set xy to position of UI element 1 of r
set wh to size of UI element 1 of r
return ((item 1 of xy) + (item 1 of wh) / 2) as text & "," & (((item 2 of xy) + (item 2 of wh) / 2) as text)
end if
end try
end repeat
return "missing"
end tell'''.replace('TARGET',target)
        point=ax(select);assert point!='missing'
        x,y=map(float,point.split(','));mouse_click(x,y)
        tone=base/'tone.wav'
        with wave.open(str(tone),'wb') as f:
            f.setnchannels(2);f.setsampwidth(2);f.setframerate(44100)
            pcm=b''.join(struct.pack('<hh',v,v) for i in range(44100*30) if (v:=int(32*math.sin(2*math.pi*440*i/44100))) or True)
            f.writeframes(pcm)
        if os.environ.get('NEONMIX_SOURCE_INSPECT')=='1':
            print('Native PIN inspection window is open for 60 seconds.',flush=True)
            time.sleep(60)
        time.sleep(2)
        report['pairing_ui_after_selection']=pairing_ui_snapshot()
        report['pin_ui_results']=[]
        # A source-owned native pairing dialog may appear in either UI process.
        pairing_deadline=time.monotonic()+65
        while time.monotonic()<pairing_deadline:
            state=api()
            if state['active']:break
            pin=state.get('pairing_pin',ready.get('pairing_pin'))
            if pin:
                prompt='''tell application "System Events" to tell process "AirPlayUIAgent"
set w to window 1
if not (name of static text 1 of w contains "TARGET_NAME") then return "other"
set frontmost to true
if exists checkbox "Remember Password" of w then
if value of checkbox "Remember Password" of w is 1 then click checkbox "Remember Password" of w
end if
set value of attribute "AXFocused" of text field 1 of w to true
keystroke "a" using command down
keystroke "PIN_VALUE"
click button "OK" of w
return "entered"
end tell'''.replace('PIN_VALUE',pin).replace('TARGET_NAME',target)
                try:result=ax(prompt)
                except RuntimeError as error:result=str(error)
                if result not in report['pin_ui_results']:report['pin_ui_results'].append(result)
            time.sleep(.15)
        report['pairing_ui_after_wait']=pairing_ui_snapshot()
        state=api();report['connected']=bool(state['active']);report['protocol_events']=state.get('protocol_events',[]);report['playback_allowed']=state['playback_allowed'];report['receiver_error']=state['error'];report['ingress']=state['ingress']
        if state['active']:
            source=subprocess.Popen([str(DEV),'/usr/bin/afplay',str(tone)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,cwd=ROOT)
            time.sleep(6)
            status=api();diagnostics=api('/v1/diagnostics')
            report['received_blocks']=status['received_blocks'];report['released_blocks']=status['released_blocks'];report['rejected_blocks']=status['rejected_blocks'];report['ingress']=status['ingress']
            binding=airplay_session(api('/v2/airplay'),status['source_id'])
            assert binding is not None,'active source has no v2 session binding'
            meter=session_meter(diagnostics,binding)
            assert meter is not None,'active source meter belongs to a different stream'
            report['airplay_binding']={key:binding[key] for key in ['session_id','stream_id','lane']}
            report['airplay_rms']=meter['rms']
            report['audio_played']=status['released_blocks']>0 and report['airplay_rms']>0
            report['passed']=report['audio_played']
    except Exception as error:
        report['failure']=str(error) if isinstance(error,AssertionError) else type(error).__name__
    finally:
        try:
            ax("""tell application "System Events" to tell process "AirPlayUIAgent"
if exists window 1 then
        if name of static text 1 of window 1 contains "TARGET" then
        if exists button "Cancel" of window 1 then click button "Cancel" of window 1
        end if
end if
end tell""".replace('TARGET',target or '__no_owned_fixture__'))
        except RuntimeError:pass
        if agent_visibility is not None:
            try:ax('tell application "System Events" to tell process "AirPlayUIAgent" to set visible to '+agent_visibility)
            except RuntimeError:report['restore_agent_visibility_failed']=True
        try:
            controls.restore_default_outputs(original)
            report['default_outputs_restored']=controls.default_outputs()==original
        except Exception:report['restore_default_output_failed']=True
        for process in [source,capture,hub]:
            if process and process.poll() is None:
                process.terminate()
                try:process.wait(timeout=8)
                except subprocess.TimeoutExpired:process.kill();process.wait()
        if log:log.close()
        remove_owned_fixture(base)
        report['fixture_files_removed']=not base.exists()
        evidence_name=os.environ.get('NEONMIX_SOURCE_EVIDENCE','mac-system-source.json')
        if Path(evidence_name).name!=evidence_name:raise ValueError('Evidence name must be a filename')
        dest=ROOT/'docs/evidence/airplay'/evidence_name;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps(report,ensure_ascii=False),flush=True)
    return 0 if report['passed'] else 1
if __name__=='__main__':sys.exit(main())
