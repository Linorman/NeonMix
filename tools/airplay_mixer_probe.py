#!/usr/bin/env python3
"""macOS synthetic encrypted AirPlay PCM through a real Hub/worker/timed Mixer.

Uses the real random PIN, SRP-20, signed pair-verify, synthetic FairPlay envelope,
NTP and encrypted RTP; not an Apple-device interoperability or latency test.
No PCM is recorded or saved. All transient data stays inside the project.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import signal
import socket
import ssl
import struct
import subprocess
import sys
import threading
import time
import traceback
from urllib.error import HTTPError
from urllib.request import Request, urlopen
import uuid
from airplay_hub_probe import airplay_session, child_workers, session_meter, wait_for

from credential_fixture import receiver_snapshot, remove_owned_fixture

ROOT=Path(__file__).resolve().parents[1]
DEV=ROOT/'tools/dev'
REPORT=ROOT/'docs/evidence/airplay/mixer-macos.json'
sys.path.insert(0,str(ROOT/'apps/airplay-worker'))
from auth_probe import Crypto, pin_setup, pair_verify, sha


def ntp(ns):
    return (((ns//1_000_000_000)+2208988800)<<32)|((ns%1_000_000_000)*(1<<32)//1_000_000_000)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile',choices=['debug','release'],default='debug')
    parser.add_argument('--output',default='coreaudio:BlackHole2ch_UID')
    parser.add_argument('--native-start-order',choices=['prewarm','hot'],default='prewarm')
    parser.add_argument('--prefetch-blocks',type=int,default=0,choices=range(0,97),metavar='0..96')
    parser.add_argument('--codec',choices=['pcm','alac'],default='pcm')
    parser.add_argument('--playback-mode',choices=['low_latency','synchronized'],default='low_latency')
    parser.add_argument('--protocol-lead-ms',type=int,default=2000,choices=range(200,3001),metavar='200..3000')
    parser.add_argument('--steady-seconds',type=float,default=3.0)
    parser.add_argument('--pause-seconds',type=float,default=0.0)
    parser.add_argument('--pause-cycles',type=int,default=1,choices=range(1,11))
    parser.add_argument('--resume-lead-ms',type=int,default=250)
    parser.add_argument('--resume-reset',choices=['none','setup','flush'],default='none')
    parser.add_argument('--track-gap',action='store_true',help='Advance RTP through the pause without FLUSH or changing its NTP mapping')
    parser.add_argument('--report',type=Path,default=REPORT)
    args=parser.parse_args()
    assert 0.25<=args.steady_seconds<=60,'steady duration must be 0.25..60 seconds'
    assert 0<=args.pause_seconds<=30 and 50<=args.resume_lead_ms<=3000,'invalid pause fixture limits'
    assert not args.track_gap or (args.pause_seconds>0 and args.resume_reset=='none'),'track gap requires a pause and no reset'
    assert platform.system()=='Darwin','macOS-only probe'
    binary=ROOT/'target'/args.profile/'neonmix-hub'
    worker=binary.with_name('neonmix-airplay-worker')
    base=ROOT/'.local/tmp'/f'airplay-mixer-{uuid.uuid4().hex[:12]}'
    base.mkdir(mode=0o700,parents=True)
    state=base/'profile';state.mkdir(mode=0o700)
    report={'platform':'macOS','build_profile':args.profile,'output_device':args.output,'native_start_order':args.native_start_order,'prefetch_blocks':args.prefetch_blocks,'codec':args.codec,
            'playback_mode':args.playback_mode,'protocol_lead_ms':args.protocol_lead_ms,
            'pause_seconds':args.pause_seconds,'pause_cycles':args.pause_cycles,'resume_lead_ms':args.resume_lead_ms,'resume_reset':args.resume_reset,
            'track_gap':args.track_gap,
            'scope':'real Hub/worker with synthetic authenticated encrypted AirPlay audio and a native SRTP Sender; real CoreAudio output; in-memory Mixer meter evidence',
            'credential_scope':'explicit init laboratory bearer tokens; receiver private file-store fixture entry',
            'binary_sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [binary,worker]},
            'passed':False,'scenarios':{},
            'limitations':['synthetic FairPlay envelope, no Apple-device interoperability claim','no saved recording or analog/audio-video latency measurement','functional debug/release result, not a sustained performance test']}
    processes=[];handles=[];sockets=[];threads=[];stop=threading.Event();errors=[];phase='initialization';hub=None;api=None;active_binding=None
    def spawn(*arguments):
        path=base/f'process-{len(processes)}.log';handle=path.open('w');handles.append(handle)
        env=os.environ.copy();env['NEONMIX_AIRPLAY_RUNTIME']=str(ROOT/'.local/airplay')
        proc=subprocess.Popen([str(DEV),str(binary),*map(str,arguments)],stdout=handle,stderr=handle,cwd=ROOT,env=env)
        processes.append(proc);return proc,path
    def open_udp():
        sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);sock.bind(('127.0.0.1',0));sockets.append(sock);return sock
    def root(value):return plistlib.dumps(value,fmt=plistlib.FMT_BINARY)
    try:
        alac_packet=None
        if args.codec=='alac':
            # One complete 352-frame period repeats without a waveform jump.
            # EOS produces a short ALAC packet with an explicit sample count,
            # matching the actual iPhone's negotiated spf=352. No audio is saved.
            ffmpeg=shutil.which('ffmpeg');assert ffmpeg,'read-only FFmpeg is required for the optional ALAC fixture'
            wave=b''.join(struct.pack('<hh',*(2*[int(.12*32767*math.sin(2*math.pi*i/352))])) for i in range(352))
            encoded=subprocess.run([ffmpeg,'-hide_banner','-loglevel','error','-f','s16le','-ar','44100','-ac','2',
                                    '-i','pipe:0','-map','0:a','-c:a','alac','-f','data','pipe:1'],
                                   input=wave,capture_output=True,timeout=10)
            assert encoded.returncode==0 and 0<len(encoded.stdout)<2048,'ALAC fixture encoding failed'
            alac_packet=encoded.stdout
        initialized=subprocess.run([str(DEV),str(binary),'init','--directory',str(state),'--output',args.output],capture_output=True,cwd=ROOT,timeout=30)
        assert initialized.returncode==0,'fixture init failed'
        credentials={name:json.loads((state/f'{name}.json').read_text()) for name in ['admin','sender-a']}
        tls=ssl.create_default_context(cadata=credentials['admin']['certificate']);tls.minimum_version=ssl.TLSVersion.TLSv1_3
        hub,log=spawn('serve','--config',state/'server.json','--listen','127.0.0.1:0')
        def started():
            assert hub.poll() is None,'fixture Hub exited'
            for line in log.read_text().splitlines():
                try:event=json.loads(line)
                except ValueError:continue
                if event.get('event')=='hub_started':return event['listen']
        url='https://'+wait_for('Hub listening',started)
        def request_api(path,role='admin',body=None):
            request=Request(url+path,data=None if body is None else json.dumps(body).encode(),
                            headers={'Authorization':'Bearer '+credentials[role]['token'],'Content-Type':'application/json'})
            try:
                with urlopen(request,context=tls,timeout=4) as response:return response.status,json.load(response)
            except HTTPError as error:return error.code,json.load(error)
        api=request_api
        def get(path):
            status,body=api(path);assert status==200,'authenticated fixture GET failed';return body
        def airplay():return get('/v1/airplay')
        def diagnostics():return get('/v1/diagnostics')
        def native_meters(d):
            stream_ids={stream['id'] for stream in get('/v1/hub')['streams'].values()}
            return [meter for meter in d['meters']['lanes'] if meter['stream_id'] in stream_ids]
        def airplay_meter(d):return session_meter(d,active_binding)
        def command(action):
            code,_=api('/v1/airplay',body={'expected_revision':airplay()['revision'],'operation':{'action':action}})
            assert code==200,'fixture AirPlay control rejected'
        wait_for('CoreAudio output available',lambda:get('/v1/hub')['output']['available'] and diagnostics()['output_frames']>4800)
        assert get('/v1/hub')['output']['gain_db']==-12,'unexpected room gain'
        native=None
        if args.native_start_order=='prewarm':
            phase='native Sender prewarm'
            native,_=spawn('send','--credential',state/'sender-a.json','--hub',url,'--seconds',str(int(args.steady_seconds)+20),'--frequency','659')
            wait_for('native Sender prewarm',lambda:any(lane['rms']>0.0005 for lane in native_meters(diagnostics())),seconds=15)
        phase='worker readiness';command('enable')
        ready=wait_for('Speaker worker ready',lambda:(s if s['ready'] else None) if (s:=airplay()) else None,seconds=25)
        identity = receiver_snapshot(state / 'airplay/receiver.json')
        assert ready['playback_mode']=='low_latency','new receiver must default to low latency'
        mode_operation={'action':'playback_mode','mode':args.playback_mode}
        status,_=api('/v1/airplay','sender-a',{'expected_revision':ready['revision'],'operation':mode_operation})
        assert status==403,'member playback mode write accepted'
        status,_=api('/v1/airplay',body={'expected_revision':ready['revision'],'operation':mode_operation})
        assert status==200,'idle playback mode selection rejected'
        command('disable')
        wait_for('worker stopped for mode persistence',lambda:not child_workers(hub))
        # The worker returns its producer asynchronously before another enable.
        time.sleep(.2)
        command('enable')
        ready=wait_for('Speaker persisted playback mode',lambda:(s if s['ready'] else None) if (s:=airplay()) else None,seconds=25)
        assert ready['playback_mode']==args.playback_mode,'playback mode did not survive receiver restart'
        current_identity = receiver_snapshot(state / 'airplay/receiver.json')
        assert current_identity[1] == identity[1], 'receiver key changed on restart'
        for field in ['key_reference', 'known_keys', 'blocked_keys', 'playback_allowed']:
            assert current_identity[0].get(field) == identity[0].get(field), 'receiver trust changed on restart'
        report['scenarios']['playback_mode_admin_permission_and_persistence']=True
        report['scenarios']['receiver_key_and_trust_survive_worker_restart']=True
        pin=ready.get('pairing_pin');assert isinstance(pin,str) and len(pin)==4,'random admin PIN missing'
        workers=wait_for('fixture worker PID',lambda:child_workers(hub));assert len(workers)==1,'unexpected worker count'
        worker_pid=workers[0]
        listing=subprocess.run(['/usr/sbin/lsof','-a','-p',str(worker_pid),'-iTCP','-sTCP:LISTEN','-Fn'],capture_output=True,text=True,check=True)
        ports={int(match[1]) for line in listing.stdout.splitlines() if line.startswith('n') and (match:=re.search(r':(\d+)$',line))}
        assert len(ports)==1,'expected one public worker RTSP listener'
        control=socket.create_connection(('127.0.0.1',next(iter(ports))),timeout=4);sockets.append(control);sequence=0
        def rtsp(method,path,body=b''):
            nonlocal sequence
            sequence+=1
            content='Content-Type: application/x-apple-binary-plist\r\n' if body else ''
            control.sendall(f'{method} {path} RTSP/1.0\r\nCSeq: {sequence}\r\nContent-Length: {len(body)}\r\n{content}\r\n'.encode()+body)
            reply=b''
            while b'\r\n\r\n' not in reply:
                data=control.recv(4096);assert data,'RTSP closed before response';reply+=data
            header,response=reply.split(b'\r\n\r\n',1);length=0
            for line in header.split(b'\r\n')[1:]:
                if line.lower().startswith(b'content-length:'):length=int(line.split(b':',1)[1])
            assert length<=16*1024,'RTSP reply exceeded probe limit'
            while len(response)<length:
                data=control.recv(4096);assert data,'RTSP response truncated';response+=data
            return int(header.split(b' ')[1]),response[:length]
        code,body=rtsp('GET','/info');info=plistlib.loads(body);assert code==200 and len(info['pk'])==32,'speaker public key missing'
        phase='PIN and signed verification';crypto=Crypto();private=os.urandom(32)
        assert rtsp('POST','/pair-pin-start')[0]==200,'PIN start rejected'
        public=pin_setup(rtsp,crypto,pin,private,proof_bytes=20,expected_pin=pin)
        secret=pair_verify(rtsp,crypto,private,public)
        pin=None
        assert rtsp('POST','/feedback')[0]==200,'signed pair-verify did not authorize feedback'
        fp=bytearray(164);fp[4]=3
        assert rtsp('POST','/fp-setup',bytes(fp))[0]==200,'FairPlay fixture envelope rejected'
        phase='timing and audio admission';timing=open_udp();timing.settimeout(0.2);timing_seen=threading.Event()
        def reply_timing():
            while not stop.is_set():
                try:data,peer=timing.recvfrom(128)
                except socket.timeout:continue
                except OSError:break
                now=ntp(time.time_ns());timing.sendto(b'\x80\xd3\x00\x07'+bytes(4)+data[24:32]+struct.pack('>QQ',now,now),peer);timing_seen.set()
        t=threading.Thread(target=reply_timing,daemon=True);threads.append(t);t.start()
        setup={'eiv':bytes(16),'ekey':bytes(72),'deviceID':'11:22:33:44:55:66','name':'Synthetic encrypted PCM probe',
               'timingProtocol':'NTP','timingPort':timing.getsockname()[1]}
        assert rtsp('SETUP','rtsp://receiver/audio',root(setup))[0]==200,'Hub audio admission rejected'
        assert timing_seen.wait(3),'no NTP exchange'
        source_port=open_udp()
        code,body=rtsp('SETUP','rtsp://receiver/audio',root({'streams':[{'type':96,'ct':2 if args.codec=='alac' else 1,'spf':352,'audioFormat':4,'controlPort':source_port.getsockname()[1]}]}))
        assert code==200,'PCM stream setup rejected'
        ports=plistlib.loads(body)['streams'][0]
        wait_for('Hub session active',lambda:airplay()['active'])
        connected=airplay()
        active_binding=wait_for('v2 AirPlay session and lane binding',
            lambda:airplay_session(get('/v2/airplay'),connected['source_id']))
        report['airplay_binding']={key:active_binding[key] for key in ['session_id','stream_id','lane']}
        assert connected['source_id'] and connected['source_name']==setup['name'],'room device source identity missing'
        assert not connected['source_revoked'],'new connection marked revoked'
        native_ids=set(get('/v1/hub')['devices'])
        assert connected['source_id'] not in native_ids,'AirPlay source incorrectly acquired a room principal'
        status,_=api('/v1/airplay',body={'expected_revision':connected['revision'],'operation':mode_operation})
        assert status==409,'mode changed under an active time line'
        report['scenarios']['room_source_identity_visible_without_control_credentials']=True
        report['scenarios']['random_pin_srp20_signed_verify_fairplay_and_hub_admission']=True
        phase='encrypted PCM to timed Mixer';key_audio=sha(crypto.fairplay(bytes(fp),bytes(72))+secret)[:16]
        target=time.time_ns()+args.protocol_lead_ms*1_000_000;rtp=0xffffff00
        sync=b'\x90\xd4\x00\x04'+struct.pack('>IQI',rtp,ntp(target),(rtp+352)&0xffffffff)
        source_port.sendto(sync,('127.0.0.1',ports['controlPort']));time.sleep(0.05)
        sent=[0];rtp_gap=[0];pause_requested=threading.Event();paused=threading.Event();resume=threading.Event()
        def send_audio():
            began=time.monotonic()
            try:
                for index in range(math.ceil((args.steady_seconds+15)*44100/352)):
                    if stop.is_set():break
                    if pause_requested.is_set():
                        gap_started=time.monotonic()
                        paused.set()
                        while not resume.wait(.05):
                            if stop.is_set():return
                        began=time.monotonic()-index*352/44100
                        if args.track_gap:
                            rtp_gap[0]+=round((time.monotonic()-gap_started)*44100)
                        pause_requested.clear();paused.clear()
                    position=index*352
                    if alac_packet is None:
                        samples=[int(0.12*32767*math.sin(2*math.pi*440*(position+frame)/44100)) for frame in range(352)]
                        payload=b''.join(struct.pack('<hh',sample,sample) for sample in samples)
                    else:
                        payload=alac_packet
                    # Classic RAOP encrypts only complete CBC blocks. The final
                    # partial compressed block remains clear on the wire.
                    aligned=len(payload)//16*16
                    encrypted=crypto.cipher('probe_cbc',payload[:aligned],key_audio,bytes(16))+payload[aligned:]
                    packet=b'\x80\x60'+struct.pack('>HII',index,(rtp+position+rtp_gap[0])&0xffffffff,0)+encrypted
                    source_port.sendto(packet,('127.0.0.1',ports['dataPort']));sent[0]+=1
                    # Send initial packets at 500pps, then preserve the original
                    # source clock. This exercises startup prefetch without
                    # overflowing the upstream RTP reorder window in one syscall.
                    deadline=began+(index+1)*(0.002 if index<args.prefetch_blocks else 352/44100)
                    stop.wait(max(0,deadline-time.monotonic()))
            except Exception as error:errors.append(type(error).__name__)
        audio_thread=threading.Thread(target=send_audio,daemon=True);threads.append(audio_thread);audio_thread.start()
        def airplay_pcm_ready():
            s=airplay();d=diagnostics();lane=airplay_meter(d)
            return (s,d) if s['released_blocks']>=20 and s['ingress']['accepted_packets']>=20 and lane and lane['rms']>0.005 else None
        first,first_d=wait_for('encrypted AirPlay PCM rendered by timed lane',airplay_pcm_ready,seconds=12)
        assert first['ingress']['identity_rejections']==0,'dynamic Hub IPC identity rejected'
        assert first['ingress']['last_source_rate']==44100,'original source rate lost'
        assert isinstance(first['format'],dict),'negotiated audio format is missing despite rendered PCM'
        assert first['format']['codec']==('alac' if args.codec=='alac' else 'pcm_s16'),'negotiated codec not preserved'
        assert first['ingress']['last_source_sample_position']>0,'original source position lost'
        assert first['ingress']['last_mapping_id']>0,'mapped speaker deadline missing'
        if args.pause_seconds:
            report['pause_resume_cycles']=[]
            for cycle in range(args.pause_cycles):
                phase='track gap with continuous RTP/NTP mapping' if args.track_gap else 'pause and resume with changed protocol lead'
                resume.clear();paused.clear();pause_requested.set();assert paused.wait(2),'synthetic source did not pause'
                stop.wait(args.pause_seconds)
                assert rtsp('POST','/feedback')[0]==200,'pause lost authenticated control connection'
                paused_status=airplay();assert paused_status['active'],'pause ended the receiver session'
                resume_position=sent[0]*352
                if args.resume_reset=='setup':
                    code,body=rtsp('SETUP','rtsp://receiver/audio',root({'streams':[{'type':96,'ct':2 if args.codec=='alac' else 1,'spf':352,'audioFormat':4,'controlPort':source_port.getsockname()[1]}]}))
                    assert code==200,'same-codec resume SETUP rejected'
                    ports=plistlib.loads(body)['streams'][0]
                    assert 0<ports['dataPort']<=65535 and 0<ports['controlPort']<=65535,'resume SETUP returned invalid audio ports'
                elif args.resume_reset=='flush':
                    assert rtsp('FLUSH','rtsp://receiver/audio')[0]==200,'resume FLUSH rejected'
                resume_rtp=(rtp+resume_position)&0xffffffff
                resume_target=time.time_ns()+args.resume_lead_ms*1_000_000
                if not args.track_gap:
                    source_port.sendto(b'\x80\xd4\x00\x04'+struct.pack('>IQI',resume_rtp,ntp(resume_target),(resume_rtp+352)&0xffffffff),('127.0.0.1',ports['controlPort']))
                time.sleep(.05);resume.set()
                def resumed_pcm():
                    s=airplay();d=diagnostics()
                    return (s,d) if s['released_blocks']>paused_status['released_blocks']+20 and airplay_meter(d) and airplay_meter(d)['rms']>.005 else None
                resumed,resumed_d=wait_for('same-session resumed audio becomes audible',resumed_pcm,seconds=5)
                assert resumed['source_id']==connected['source_id'] and resumed['active'],'resume changed source ownership'
                report['pause_resume']={'released_before':paused_status['released_blocks'],'released_after':resumed['released_blocks'],
                    'late_packets':resumed['ingress']['late_packets'],'timeline_rejections':resumed['ingress']['timeline_rejections'],
                    'media_resets':resumed['media_resets'],'last_media_reset':resumed['last_media_reset'],
                    'lane_rms':airplay_meter(resumed_d)['rms'],'timed_late_frames':resumed_d['timed_late_frames']}
                if args.track_gap:
                    assert resumed['media_resets']==0,'track gap unexpectedly reset the media session'
                    # audioresample releases its retained old-track tail only
                    # when the next packet arrives. Across a long gap that one
                    # stale chunk must be discarded, never replayed as new PCM.
                    late_delta=resumed['ingress']['late_packets']-paused_status['ingress']['late_packets']
                    assert 0<=late_delta<=1,'more than the retained decoder tail expired at the track boundary'
                    assert resumed['rejected_blocks']-paused_status['rejected_blocks']==late_delta,'track gap rejected non-expired media'
                    report['scenarios']['track_gap_recovers_without_flush_setup_or_epoch_reset']=True
                report['pause_resume_cycles'].append(report['pause_resume'].copy())
                report['scenarios']['same_session_pause_resume_without_route_reselection']=True
                first,first_d=resumed,resumed_d
        time.sleep(args.steady_seconds)
        second,second_d=wait_for('PCM counters remain continuous',lambda:(result if result[0]['released_blocks']>first['released_blocks'] else None) if (result:=airplay_pcm_ready()) else None)
        assert second['ingress']['identity_rejections']==0 and not errors,'encrypted PCM transport failed'
        if args.track_gap:
            assert second['rejected_blocks']==first['rejected_blocks'] and second['ingress']['late_packets']==first['ingress']['late_packets'],'steady audio still expires after track recovery'
        else:
            assert second['rejected_blocks']==0 and second['ingress']['late_packets']==0,'retiming rejected media'
        assert second_d['timed_late_frames']==0,'retiming skipped output samples'
        if not args.pause_seconds:
            assert second['media_resets']==0,'normal startup/prefetch unexpectedly reset media'
        timing_stats=second['ingress']
        if args.playback_mode=='low_latency' and not args.pause_seconds:
            assert timing_stats['latency_advance_ns']>1_000_000_000 if args.protocol_lead_ms==2000 else timing_stats['latency_advance_ns']>0,'low latency did not advance source time'
            # Synthetic source cadence and the fixed mapping preserve headroom.
            assert 50_000_000<timing_stats['playout_lead_ns']<180_000_000,'low latency headroom not bounded'
        elif args.playback_mode=='synchronized':
            assert timing_stats['latency_advance_ns']==0,'synchronized mode changed protocol PTS'
            expected_lead=args.resume_lead_ms if args.pause_seconds and not args.track_gap else args.protocol_lead_ms
            assert timing_stats['playout_lead_ns']>expected_lead*1_000_000-200_000_000,'source sync lead lost'
        report['timing']={key:timing_stats[key] for key in ['latency_advance_ns','protocol_lead_ns','playout_lead_ns','output_latency_ns']}
        report['scenarios']['steady_timing_without_new_late_packets_or_skipped_samples']=True
        report['scenarios']['dynamic_hub_context_normalized_pcm_released_to_timed_mixer']=True
        report['airplay_pcm']={'received_blocks':second['received_blocks'],'released_blocks':second['released_blocks'],
                               'codec':second['format']['codec'],'source_frame_count':second['format']['source_frame_count'],
                               'accepted_packets':second['ingress']['accepted_packets'],'identity_rejections':second['ingress']['identity_rejections'],
                               'source_rate':second['ingress']['last_source_rate'],'internal_rate':48000,'lane_rms':airplay_meter(second_d)['rms'],
                               'uncertainty_ns':second['ingress']['last_uncertainty_ns'],'timed_late_frames':second_d['timed_late_frames']}
        phase='native and AirPlay mixing'
        if native is None:
            native,_=spawn('send','--credential',state/'sender-a.json','--hub',url,'--seconds',str(int(args.steady_seconds)+20),'--frequency','659')
        def mixed():
            d=diagnostics();meter=airplay_meter(d)
            return d if meter and meter['rms']>0.005 and any(lane['rms']>0.0005 for lane in native_meters(d)) and d['meters']['output']['rms']>0.001 else None
        mixed_d=wait_for('native plus encrypted AirPlay rendered concurrently',mixed,seconds=12)
        report['scenarios']['encrypted_airplay_and_native_sender_one_plus_one_mixing']=True
        report['mixed_output_rms']=mixed_d['meters']['output']['rms']
        phase='worker crash isolation';assert worker_pid in child_workers(hub),'worker ownership changed'
        before=mixed_d['output_frames'];os.kill(worker_pid,signal.SIGKILL)
        stop.set();audio_thread.join(1)
        wait_for('worker crash recognized',lambda:not airplay()['enabled'] and airplay().get('error')=='worker_failed')
        def native_continues():
            assert hub.poll() is None and native.poll() is None,'worker crash stopped Hub/native Sender'
            d=diagnostics();retired=d['meters']['lanes'][active_binding['lane']]
            return d if d['output_frames']>before+4800 and retired['stream_id']==0 and retired['rms']<1e-6 and any(lane['rms']>0.0005 for lane in native_meters(d)) else None
        continued=wait_for('native output persists while AirPlay lane clears',native_continues)
        report['scenarios']['worker_kill_clears_airplay_and_preserves_native_audio']=True
        report['output_frames_around_worker_crash']={'before':before,'after':continued['output_frames']}
        report['passed']=True
    except Exception as error:
        report['failure_phase']=phase;report['failure_type']=type(error).__name__
        report['failure_location']=[{'file':Path(frame.filename).name,'line':frame.lineno} for frame in traceback.extract_tb(error.__traceback__)]
        # Only this script's static assertions may be printed, never helper bytes/PIN.
        if isinstance(error,AssertionError) and error.__traceback__ and error.__traceback__.tb_next is None:report['failure']=str(error)
        if api:
            try:
                s=api('/v1/airplay')[1]
                report['failure_counters']={key:s.get(key) for key in ['enabled','ready','active','error','failure_stage','received_blocks','rejected_blocks','released_blocks','format','ingress']}
                d=api('/v1/diagnostics')[1]
                report['failure_output']={'lane_rms':meter['rms'] if (meter:=session_meter(d,active_binding)) else None,
                    'timed_late_frames':d['timed_late_frames']}
            except Exception:pass
    finally:
        stop.set()
        for thread in threads:thread.join(timeout=1)
        for sock in sockets:
            try:sock.close()
            except OSError:pass
        if api:
            try:
                s=api('/v1/airplay')[1];api('/v1/airplay',body={'expected_revision':s['revision'],'operation':{'action':'disable'}})
            except Exception:pass
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try:process.wait(timeout=10)
                except subprocess.TimeoutExpired:process.kill();process.wait(timeout=5)
        for handle in handles:handle.close()
        report['fixture_processes_stopped']=all(process.poll() is not None for process in processes)
        remove_owned_fixture(base)
        report['fixture_files_removed']=not base.exists()
        args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n')
    print(json.dumps(report,ensure_ascii=False))
    return 0 if report['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
