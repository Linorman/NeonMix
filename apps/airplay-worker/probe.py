#!/usr/bin/env python3
"""macOS worker startup / speaker policy probe. No Apple interoperability claims."""
import atexit, queue, json, os, plistlib, socket, struct, subprocess, sys, threading, time
from auth_probe import Crypto,pin_setup,pair_verify,sha
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
BASE=ROOT/'.local/airplay'
def main():
    if sys.platform!='darwin':raise SystemExit('macOS only')
    state=BASE/'probe';state.mkdir(exist_ok=True);key=state/'identity.pem'
    subprocess.run(['/opt/homebrew/opt/openssl@3/bin/openssl','genpkey','-algorithm','ED25519','-out',str(key)],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);key.chmod(0o600)
    media=socket.socket();media.bind(('127.0.0.1',0));media.listen(1)
    config=dict(control_version=2,worker_generation=7,trust_generation=1,pairing_remaining_ms=600000,pairing_attempts=0,media_address=f'127.0.0.1:{media.getsockname()[1]}',ipc_token='a'*64,device_id='001122334455',receiver_uuid='00112233-4455-6677-8899-aabbccddeeff',keyfile=str(key),name='NeonMix probe',pin='1234',rtsp_port=0,session_id=1,stream_id=2,stream_epoch=1,format_epoch=1,mapping_id=1,pairing_allowed=True,known_client_keys=[],blocked_client_keys=[])
    config['protocol_trace']=os.environ.get('NEONMIX_AIRPLAY_PROBE_TRACE')=='1'
    env=os.environ.copy();env['GST_PLUGIN_SYSTEM_PATH_1_0']=str(BASE/'plugins');env['GST_REGISTRY']=str(BASE/'probe-registry.bin')
    process=subprocess.Popen([os.environ.get('NEONMIX_AIRPLAY_WORKER',str(BASE/'build/neonmix-airplay-worker'))],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    # libplist JSON saturates oversized signed integers; reject before opening IPC.
    for bad_identity in [(1<<53), (1<<64)-1, -1]:
        invalid=config|dict(session_id=bad_identity)
        rejected=subprocess.run([os.environ.get('NEONMIX_AIRPLAY_WORKER',str(BASE/'build/neonmix-airplay-worker'))],env=env,input=json.dumps(invalid)+'\n',text=True,capture_output=True,timeout=3)
        assert rejected.returncode==1 and json.loads(rejected.stdout)['type']=='fatal'
        assert '2^53-1' in json.loads(rejected.stdout)['message']
    def cleanup():
        if process.poll() is None:
            process.terminate()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()
        key.unlink(missing_ok=True)
        (BASE/'probe-registry.bin').unlink(missing_ok=True)
        if state.exists():state.rmdir()
    atexit.register(cleanup)
    command_lock=threading.Lock()
    def command(msg):
        with command_lock:process.stdin.write(json.dumps(msg)+'\n');process.stdin.flush()
    command(config)
    media.settimeout(10);connection,_=media.accept();connection.settimeout(10);token=b''
    while not token.endswith(b'\n'):token+=connection.recv(1)
    assert token==b'a'*64+b'\n'
    events=queue.Queue();transport_events=[];other_diagnostics=[];pairing_attempts=[];authorized_attempts=[0]
    def read_events():
        for line in process.stdout:
            assert len(line)<=4096, 'diagnostic exceeds worker event limit'
            value=json.loads(line)
            if value.get('type')=='pairing_request':
                allowed=authorized_attempts[0]<5
                if allowed:authorized_attempts[0]+=1
                command(dict(type='pairing_admit',worker_generation=value['worker_generation'],trust_generation=value['trust_generation'],connection_id=value['connection_id'],pairing_request_id=value['pairing_request_id'],allowed=allowed,attempts=authorized_attempts[0]))
            elif value.get('type')=='pairing_ended':pass
            elif value.get('type')=='protocol_transport':transport_events.append(value)
            elif value.get('type')=='pairing_attempt':pairing_attempts.append(value)
            elif value.get('type') in ['protocol','protocol_detail']:other_diagnostics.append(value)
            else:events.put(value)
    event_reader=threading.Thread(target=read_events,daemon=True);event_reader.start()
    def event(skip_formats=True):
        while True:
            try:value=events.get(timeout=10)
            except queue.Empty:raise RuntimeError(f"worker event timeout, process={process.poll()}")
            if not skip_formats or value.get('type')!='format':return value
    first=event();assert first['type']=='ready' and first['control_version']==2 and first['worker_generation']==7,first
    assert first['features']==0x481C5A00 and len(first['public_key'])==64
    client=socket.create_connection(('127.0.0.1',first['port']),timeout=3);sequence=0
    def request(method,url,body=b'',protocol='RTSP/1.0',sock=client,fragment=False):
        nonlocal sequence
        sequence+=1
        content_type='Content-Type: application/x-apple-binary-plist\r\n' if body else ''
        message=f'{method} {url} {protocol}\r\nCSeq: {sequence}\r\nContent-Length: {len(body)}\r\n{content_type}\r\n'.encode()+body
        if fragment:
            split=message.index(b' RTSP/1.0');sock.sendall(message[:split]);time.sleep(.03);sock.sendall(message[split:])
        else:sock.sendall(message)
        data=b''
        while b'\r\n\r\n' not in data:data+=sock.recv(4096)
        header,body0=data.split(b'\r\n\r\n',1);length=0
        for line in header.split(b'\r\n')[1:]:
            if line.lower().startswith(b'content-length:'):length=int(line.split(b':',1)[1])
        while len(body0)<length:body0+=sock.recv(4096)
        return int(header.split(b' ')[1]),body0[:length]
    assert request('POST','/pair-verify',bytes([1,0,0,0])+bytes(64))[0]==400
    assert request('OPTIONS','*',fragment=True)[0]==200
    code,body=request('GET','/info');assert code==200
    info=plistlib.loads(body);assert info['features']==first['features'] and 'displays' not in info and info['statusFlags']==68 and info['pi']==config['receiver_uuid']
    code,body=request('GET','/info',plistlib.dumps(dict(qualifier=['txtRAOP']),fmt=plistlib.FMT_BINARY));assert code==200
    txt=plistlib.loads(body)['txtRAOP'];txt_fields={};offset=0
    while offset<len(txt):
        length=txt[offset];item=txt[offset+1:offset+1+length].decode();offset+=length+1
        key0,value0=item.split('=',1);txt_fields[key0]=value0
    assert all(txt_fields[key0]==value0 for key0,value0 in dict(model='AppleTV3,2',srcvers='220.68',da='true',sv='false',sf='0x4',pw='true',ft='0x481C5A00,0x0').items())
    code,body=request('GET','/info',plistlib.dumps(dict(qualifier=['txtAirPlay']),fmt=plistlib.FMT_BINARY));assert code==200
    qualified=plistlib.loads(body);assert set(qualified)=={'txtAirPlay'}
    txt=qualified['txtAirPlay'];txt_fields={};offset=0
    while offset<len(txt):
        length=txt[offset];item=txt[offset+1:offset+1+length].decode();offset+=length+1
        key0,value0=item.split('=',1);txt_fields[key0]=value0
    assert all(txt_fields[key0]==value0 for key0,value0 in dict(model='AppleTV3,2',srcvers='220.68',pi=config['receiver_uuid'],flags='0x4',pw='true',features='0x481C5A00,0x0',pk=first['public_key']).items())
    assert request('POST','/play',b'Content-Location: http://invalid.example/video',protocol='HTTP/1.1')[0]==415
    assert request('TEARDOWN','rtsp://receiver/audio',plistlib.dumps(dict(streams=[dict(type=110)]),fmt=plistlib.FMT_BINARY))[0]==415
    assert request('POST','/feedback')[0]==403
    for stream in [dict(type=110),dict(type=103),dict(type=96,ct=2,usingScreen=True),dict(type=96,ct=2,audioFormat=8)]:
        assert request('SETUP','rtsp://receiver/audio',plistlib.dumps(dict(streams=[stream]),fmt=plistlib.FMT_BINARY))[0]==415
    assert request('SETUP','rtsp://receiver/audio',plistlib.dumps(dict(eiv=b'0'*16,ekey=b'0'*72),fmt=plistlib.FMT_BINARY))[0]==403
    second=socket.create_connection(('127.0.0.1',first['port']),timeout=3)
    assert request('OPTIONS','*',sock=second)[0]==409;second.close()
    assert request('POST','/pair-pin-start')[0]==200;pin=event();assert pin==dict(type='pairing_pin',worker_generation=7,pin='1234')
    root=lambda v:plistlib.dumps(v,fmt=plistlib.FMT_BINARY)
    # Malformed and out-of-order SRP steps fail without a dereference/crash.
    assert request('POST','/pair-setup-pin',root(dict(epk=bytes(1),authTag=bytes(1))))[0]==470
    crypto=Crypto();private=os.urandom(32)
    pin_setup(request,crypto,'9999',private)
    public=pin_setup(request,crypto,'1234',private)
    registered=event();assert registered['type']=='registered'
    secret=pair_verify(request,crypto,private,public)
    assert request('POST','/feedback')[0]==200
    # Synthetic FairPlay envelope exercises key handoff; not an Apple fixture.
    fp=bytearray(164);fp[4]=3
    assert request('POST','/fp-setup',bytes(fp))[0]==200
    timing=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);timing.bind(('127.0.0.1',0));timing.settimeout(.2)
    timing_done=threading.Event();timing_seen=threading.Event()
    def ntp(ns):return (((ns//1000000000)+2208988800)<<32)|((ns%1000000000)*(1<<32)//1000000000)
    def timing_reply():
        while not timing_done.is_set():
            try:data,addr=timing.recvfrom(128)
            except socket.timeout:continue
            now=ntp(time.time_ns());reply=b'\x80\xd3\x00\x07'+bytes(4)+data[24:32]+struct.pack('>QQ',now,now);timing.sendto(reply,addr);timing_seen.set()
    thread=threading.Thread(target=timing_reply,daemon=True);thread.start()
    setup=dict(eiv=bytes(16),ekey=bytes(72),deviceID='11:22:33:44:55:66',name='Synthetic sender',timingProtocol='NTP',timingPort=timing.getsockname()[1])
    result=[];setupthread=threading.Thread(target=lambda:result.append(request('SETUP','rtsp://receiver/audio',root(setup))));setupthread.start()
    admission=event();assert admission['type']=='admit_request' and admission['request_id']>0
    command(dict(type='admit',worker_generation=7,connection_id=admission['connection_id'],request_id=admission['request_id'],allowed=True))
    started=event();assert started['type']=='session_started'
    # Preserve full context and grant only after the session has been accepted.
    setupthread.join(3);assert result and result[0][0]==200,result
    assert timing_seen.wait(3)
    assert request('SETUP','rtsp://receiver/audio',root(setup))[0]==455, 'duplicate initial SETUP must preserve owner'
    cport=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);cport.bind(('127.0.0.1',0))
    code,body=request('SETUP','rtsp://receiver/audio',root(dict(streams=[dict(type=96,ct=1,spf=352,audioFormat=4,controlPort=cport.getsockname()[1])])));assert code==200
    ports=plistlib.loads(body)['streams'][0]
    initial_format=event(skip_formats=False)
    assert initial_format['type']=='format' and initial_format['session_id']==0 and initial_format['stream_epoch']==0
    def grant_with_format(epoch):
        command(dict(type='grant',worker_generation=7,connection_id=started['connection_id'],request_id=started['request_id'],session_id=1,stream_id=2,stream_epoch=epoch,format_epoch=1,mapping_id=epoch))
        observed=event(skip_formats=False)
        assert observed['type']=='format' and observed['session_id']==1 and observed['stream_epoch']==epoch, observed
        assert observed['codec']=='pcm_s16' and observed['source_rate']==44100 and observed['source_frame_count']==352
        assert observed['connection_id']==started['connection_id'] and observed['request_id']==started['request_id']
        applied=event(skip_formats=False)
        assert applied['type']=='grant_applied' and applied['session_id']==1 and applied['stream_epoch']==epoch
    grant_with_format(2)
    # Repeated stream SETUP negotiates before the next Hub epoch as well.
    code,body=request('SETUP','rtsp://receiver/audio',root(dict(streams=[dict(type=96,ct=1,spf=352,audioFormat=4,controlPort=cport.getsockname()[1])])));assert code==200
    ports=plistlib.loads(body)['streams'][0]
    flushed=event(skip_formats=False);assert flushed['type']=='flush' and flushed['reason']=='stream_setup' and flushed['stream_epoch']==2
    prior_format=event(skip_formats=False);assert prior_format['type']=='format' and prior_format['stream_epoch']==2
    grant_with_format(3)
    time.sleep(.05)
    while not events.empty():
        assert event(skip_formats=False)['type']=='format'
    # A denied extra TCP connection must not end or displace the established owner.
    second=socket.create_connection(('127.0.0.1',first['port']),timeout=3)
    assert request('OPTIONS','*',sock=second)[0]==409;second.close();time.sleep(.05)
    assert events.empty(), 'extra connection ended owner'
    assert request('POST','/feedback')[0]==200
    key_audio=sha(crypto.fairplay(bytes(fp),bytes(72))+secret)[:16]
    start=time.time_ns()+300000000;rtp=0xffffff00
    sync=b'\x90\xd4\x00\x04'+struct.pack('>IQI',rtp,ntp(start),(rtp+352)&0xffffffff)
    cport.sendto(sync,('127.0.0.1',ports['controlPort']));time.sleep(.05)
    pcm=bytes(352*4);encrypted=crypto.cipher('probe_cbc',pcm,key_audio,bytes(16))
    for i in range(24):
        packet=b'\x80\x60'+struct.pack('>HII',i,(rtp+i*352)&0xffffffff,0)+encrypted;cport.sendto(packet,('127.0.0.1',ports['dataPort']));time.sleep(.003)
    connection.settimeout(3);packet=b''
    while len(packet)<112:packet+=connection.recv(112-len(packet))
    assert packet[:4]==b'NMAM' and struct.unpack_from('<I',packet,88)[0]==44100 and struct.unpack_from('<I',packet,104)[0]==48000
    assert struct.unpack_from('<Q',packet,32)[0]==3 and 0<struct.unpack_from('<H',packet,92)[0]<=480
    assert 0<struct.unpack_from('<Q',packet,80)[0]<20000000
    assert abs(struct.unpack_from('<Q',packet,64)[0]-start)<20000000
    timing_done.set();thread.join(1);timing.close();cport.close()
    client.close();assert event()['type']=='session_ended'
    # Previously PIN-bound key reconnects with a signed verify; no new PIN exchange.
    repeat=socket.create_connection(('127.0.0.1',first['port']),timeout=3)
    repeat_request=lambda *a,**kw:request(*a,sock=repeat,**kw)
    pair_verify(repeat_request,crypto,private,public)
    assert repeat_request('POST','/feedback')[0]==200
    command(dict(type='trust_update',worker_generation=7,trust_generation=2,known_client_keys=[],blocked_client_keys=[registered['client_public_key']],pairing_allowed=True))
    repeat.close();command(dict(type='allow',worker_generation=7));time.sleep(.05)
    revoked=socket.create_connection(('127.0.0.1',first['port']),timeout=3)
    xpriv=os.urandom(32);xpub=crypto.primitive('probe_x_public',xpriv)
    assert request('POST','/pair-verify',bytes([1,0,0,0])+xpub+public,sock=revoked)[0]==403
    revoked.close();time.sleep(.05)
    mismatch=socket.create_connection(('127.0.0.1',first['port']),timeout=3)
    mismatch_request=lambda *a,**kw:request(*a,sock=mismatch,**kw)
    private2=os.urandom(32);public2=pin_setup(mismatch_request,crypto,'1234',private2,proof_bytes=64)
    assert event()['type']=='registered'
    alternate=crypto.primitive('probe_ed_public',os.urandom(32));xpub=crypto.primitive('probe_x_public',os.urandom(32))
    assert mismatch_request('POST','/pair-verify',bytes([1,0,0,0])+xpub+alternate)[0]==403
    mismatch.close();time.sleep(.05)
    # No pair-pin-start is needed to attack SRP; fresh initial requests still count.
    limit=socket.create_connection(('127.0.0.1',first['port']),timeout=3)
    init=root(dict(method='pin',user='11:22:33:44:55:77'))
    assert request('POST','/pair-setup-pin',init,sock=limit)[0]==200
    assert request('POST','/pair-setup-pin',root(dict(pk=bytes(1),proof=bytes(64))),sock=limit)[0]==470
    assert request('POST','/pair-setup-pin',init,sock=limit)[0]==200
    assert request('POST','/pair-setup-pin',init,sock=limit)[0]==403
    limit.close()
    # An explicit idle window can renew the PIN and attempt budget in-place.
    authorized_attempts[0]=0
    command(dict(type='trust_update',worker_generation=7,trust_generation=3,known_client_keys=[],blocked_client_keys=[],pairing_allowed=True))
    command(dict(type='pairing_window',worker_generation=7,trust_generation=3,pin='2468',remaining_ms=600000,attempts=0))
    renewed=event();assert renewed==dict(type='pairing_window_applied',worker_generation=7,trust_generation=3)
    renewed_client=socket.create_connection(('127.0.0.1',first['port']),timeout=3)
    renewed_request=lambda *a,**kw:request(*a,sock=renewed_client,**kw)
    assert renewed_request('POST','/pair-pin-start')[0]==200
    assert event()==dict(type='pairing_pin',worker_generation=7,pin='2468')
    pin_setup(renewed_request,crypto,'2468',os.urandom(32),expected_pin='2468')
    registered_new=event();assert registered_new['type']=='registered' and registered_new['trust_generation']==3 and registered_new['pairing_request_id']>0
    renewed_client.close()
    command(dict(type='stop'));process.wait(timeout=5)
    event_reader.join(2);assert not event_reader.is_alive()
    stderr=process.stderr.read();assert process.returncode==0,stderr
    if config['protocol_trace']:
        required={'type','connection_id','event','relative_ms','protocol','method','route','body_bytes','cseq_present','session_present','status','response_bytes','sent_bytes','outcome'}
        assert transport_events
        for value in transport_events:
            assert set(value)==required, value
            assert 0<value['connection_id']<(1<<53)
            assert value['protocol'] in ['none','other','RTSP/1.0','HTTP/1.1']
            assert value['event'] in ['open','response','close']
            assert isinstance(value['cseq_present'],bool) and isinstance(value['session_present'],bool)
            assert value['outcome'] in ['opened','complete','send_error','send_closed','send_timeout','no_response','peer_closed','recv_error','parse_error','requested','software','shutdown','select_error']
        responses=[v for v in transport_events if v['event']=='response']
        assert responses and all(v['outcome']=='complete' and v['sent_bytes']==v['response_bytes']>0 for v in responses)
        pin_start=next(v for v in responses if v['route']=='pair_pin_start')
        assert pin_start['protocol']=='RTSP/1.0' and pin_start['status']==200 and pin_start['body_bytes']==0 and pin_start['cseq_present'] and not pin_start['session_present']
        assert any(v['status']==409 for v in responses)
        opened=[v['connection_id'] for v in transport_events if v['event']=='open']
        closed=[v['connection_id'] for v in transport_events if v['event']=='close']
        assert opened==sorted(set(opened)) and set(opened)==set(closed) and len(closed)==len(set(closed))
        assert any(v['outcome']=='peer_closed' for v in transport_events if v['event']=='close')
    else:assert not transport_events and not other_diagnostics
    assert [v['attempts'] for v in pairing_attempts]==[1,2,3,4,5,5,1]
    assert all(v['worker_generation']==7 for v in pairing_attempts)

    connection.close();media.close();cleanup()
    print(json.dumps(dict(worker='passed',platform='macos',trace_enabled=config['protocol_trace'],transport_events=len(transport_events),checks=['format follows granted context when SETUP arrives first','repeated stream SETUP reannounces format for the next epoch','trace opt-in and completed-send metadata','connection lifecycle IDs and close reasons','private IPC authentication','audio-only info','video/HLS/mirror rejection','unauthorized audio rejected','second client denied','PIN event','malformed SRP rejected','wrong PIN rejected','correct PIN with native 20-byte proof and signed pair verify','legacy 64-byte padded proof','second socket close preserves owner','encrypted RTP PCM reaches bounded IPC with NTP mapping','paired reconnect','revoked key rejected','PIN cannot authorize a different key','five SRP attempt limit','invalid X25519 and zero SRP keys rejected','fragmented request parsed','oversized and negative control identities rejected'],source_rate=struct.unpack_from('<I',packet,88)[0],pcm_rate=struct.unpack_from('<I',packet,104)[0],ntp_uncertainty_ns=struct.unpack_from('<Q',packet,80)[0],ntp_mapping_delta_ns=int(struct.unpack_from('<Q',packet,64)[0])-start,apple_interoperability='not tested')))
if __name__=='__main__':main()
