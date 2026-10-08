#!/usr/bin/env python3
"""Authenticated worker SETUP and UDP regression probe (no audio device).

Run through tools/dev or tools/dev.ps1. All temporary identities and registries
stay under project .local/tmp and are removed. Supply a matching worker, crypto
probe library and audio-only plugin directory. Windows also needs --dll-dir for
its matched GStreamer/MinGW runtimes. Reports contain no keys or PINs.
"""
import argparse,base64,hashlib,json,os,plistlib,socket,struct,subprocess,sys,threading,time,uuid
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--worker',required=True);p.add_argument('--crypto',required=True);p.add_argument('--openssl',required=True);p.add_argument('--report',required=True);p.add_argument('--plugins',required=True);p.add_argument('--dll-dir',action='append',default=[]);a=p.parse_args()
root=Path(__file__).resolve().parents[1];
report_path=Path(a.report).resolve();report_path.relative_to(root);report_path.parent.mkdir(parents=True,exist_ok=True)
sys.path[:0]=[str(root/'tools'),str(root/'apps/airplay-worker')]
from credential_store_probe import protect_fixture
import auth_probe
dll_handles=[os.add_dll_directory(str(Path(p).resolve())) for p in a.dll_dir] if os.name=='nt' else []
crypto=auth_probe.Crypto(Path(a.crypto).resolve())
state=root/'.local/tmp'/('setup-'+uuid.uuid4().hex);state.mkdir(parents=True)
key=state/'identity.pem'
def ntp(ns):return (((ns//1000000000)+2208988800)<<32)|((ns%1000000000)*(1<<32)//1000000000)
# 352 stereo sine frames encoded by FFmpeg ALAC; synthetic public test data.
ALAC=bytes.fromhex('200010000002c000020d0c010fff97ffca000dffec001a0f0801000000000000000ff8022ffe008cff80233d1f47d100418620428c188631080a2201054044a8644007600f4eddbb76ec762a5290aad00804e48020061c9288447723b44154952a54a9529201c02a0e0e3838e03693556548000003579469a4d915408d32509c71340954a952a54a5323b551c23881dc4087710053072ed4b908ee58a5220f4471c71c1c713409c1c1db52a308ca8937742007fc057f80')
def run_case(ct,spf,noise=False,control_port=None):
 sockets=[];proc=None;done=threading.Event();events=[];media_bytes=[0]
 result={'codec':ct,'spf':spf,'udp_noise':noise,'invalid_control_port':control_port is not None}
 def sock(kind=socket.SOCK_STREAM):
  s=socket.socket(socket.AF_INET,kind);s.setsockopt(socket.SOL_SOCKET,socket.SO_SNDBUF,131072);s.bind(('127.0.0.1',0));sockets.append(s);return s
 def await_event(typ):
  end=time.monotonic()+5
  while time.monotonic()<end:
   found=next((e for e in events if e['type']==typ),None)
   if found:return found
   if proc.poll() is not None:raise RuntimeError('worker exited '+str(proc.returncode))
   time.sleep(.01)
  raise RuntimeError('event timeout '+typ)
 try:
  media=sock();media.listen(1);media.settimeout(5)
  private=os.urandom(32);public=crypto.primitive('probe_ed_public',private)
  env=os.environ.copy();env['GST_PLUGIN_SYSTEM_PATH_1_0']=str(Path(a.plugins).resolve());env['GST_REGISTRY_FORK']='no';env['PATH']=os.pathsep.join(a.dll_dir+[env.get('PATH','')]);env['GST_REGISTRY']=str(state/'registry.bin');env['NEONMIX_AUDIO_REGISTRY']=str(state/'registry.bin')
  proc=subprocess.Popen([str(Path(a.worker).resolve())],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=env)
  lock=threading.Lock()
  def command(msg):
   with lock:proc.stdin.write(json.dumps(msg)+'\n');proc.stdin.flush()
  def read_events():
   for line in proc.stdout:
    e=json.loads(line);events.append(e)
    if e['type']=='admit_request':command(dict(type='admit',worker_generation=7,connection_id=e['connection_id'],request_id=e['request_id'],allowed=True))
  threading.Thread(target=read_events,daemon=True).start()
  command(dict(control_version=2, pcm_version=2,session_control_version=1,worker_generation=7,trust_generation=1,pairing_remaining_ms=600000,pairing_attempts=0,media_address=f'127.0.0.1:{media.getsockname()[1]}',ipc_token='a'*64,device_id='001122334455',receiver_uuid='00112233-4455-6677-8899-aabbccddeeff',keyfile=str(key),name='Setup regression',pin='1234',rtsp_port=0,session_id=1,stream_id=2,stream_epoch=1,format_epoch=1,mapping_id=1,pairing_allowed=True,known_client_keys=[base64.b64encode(public).decode()],blocked_client_keys=[]))
  channel,_=media.accept();sockets.append(channel);channel.settimeout(.2)
  def drain():
   while not done.is_set():
    try:
     data=channel.recv(65536)
     if not data:return
     media_bytes[0]+=len(data)
    except socket.timeout:pass
    except OSError:return
  threading.Thread(target=drain,daemon=True).start()
  ready=await_event('ready');control=socket.create_connection(('127.0.0.1',ready['port']),timeout=5);sockets.append(control)
  seq=0
  def request(method,url,body=b''):
   nonlocal seq
   seq+=1;control.sendall(f'{method} {url} RTSP/1.0\r\nCSeq: {seq}\r\nContent-Length: {len(body)}\r\nContent-Type: application/x-apple-binary-plist\r\n\r\n'.encode()+body)
   data=b''
   while b'\r\n\r\n' not in data:
    chunk=control.recv(4096)
    if not chunk:raise RuntimeError('RTSP closed')
    data+=chunk
   h,b=data.split(b'\r\n\r\n',1);length=next((int(l.split(b':',1)[1]) for l in h.split(b'\r\n') if l.lower().startswith(b'content-length:')),0)
   while len(b)<length:
    chunk=control.recv(4096)
    if not chunk:raise RuntimeError('RTSP body closed')
    b+=chunk
   return int(h.split(b' ')[1]),b[:length]
  secret=auth_probe.pair_verify(request,crypto,private,public)
  fp=bytearray(164);fp[4]=3;assert request('POST','/fp-setup',bytes(fp))[0]==200
  timing=sock(socket.SOCK_DGRAM);timing.settimeout(.2);timing_seen=threading.Event()
  def timer():
   while not done.is_set():
    try:
     data,addr=timing.recvfrom(128);now=ntp(time.time_ns());timing.sendto(b'\x80\xd3\x00\x07'+bytes(4)+data[24:32]+struct.pack('>QQ',now,now),addr);timing_seen.set()
    except socket.timeout:pass
    except OSError:return
  threading.Thread(target=timer,daemon=True).start()
  plist=lambda v:plistlib.dumps(v,fmt=plistlib.FMT_BINARY)
  setup=dict(eiv=bytes(16),ekey=bytes(72),deviceID='11:22:33:44:55:66',name='Synthetic',timingProtocol='NTP',timingPort=timing.getsockname()[1])
  assert request('SETUP','rtsp://receiver/audio',plist(setup))[0]==200
  started=await_event('session_started');assert timing_seen.wait(3)
  source=sock(socket.SOCK_DGRAM)
  stream=dict(type=96,ct=ct,audioFormat=4,controlPort=source.getsockname()[1])
  if spf is not None:stream['spf']=spf
  if control_port is not None:stream['controlPort']=control_port
  code,body=request('SETUP','rtsp://receiver/audio',plist(dict(streams=[stream])));result['setup_status']=code
  if code!=200:return result
  ports=plistlib.loads(body)['streams'][0];result['ports_nonzero']=all(ports[k]>0 for k in ['dataPort','controlPort'])
  fmt=await_event('format');result['source_frame_count']=fmt['source_frame_count']
  command(dict(type='grant',worker_generation=7,connection_id=started['connection_id'],request_id=started['request_id'],session_id=1,stream_id=2,stream_epoch=2,format_epoch=1,mapping_id=2));await_event('grant_applied')
  request('RECORD','rtsp://receiver/audio')
  rtp=50000;start=time.time_ns()+1000000000
  source.sendto(b'\x90\xd4\x00\x04'+struct.pack('>IQI',rtp,ntp(start),rtp+352),('127.0.0.1',ports['controlPort']));time.sleep(.05)
  payload=ALAC if ct==2 else bytes(352*4)
  key_audio=auth_probe.sha(crypto.fairplay(bytes(fp),bytes(72))+secret)[:16]
  complete=len(payload)//16*16;encrypted=crypto.cipher('probe_cbc',payload[:complete],key_audio,bytes(16))+payload[complete:]
  for i in range(1100 if spf is None else 30):
   if noise and i==15:
    source.sendto(b'\x90\xd4\x00\x04'+struct.pack('>IQI',rtp,ntp(start+3000000000),rtp+352)+bytes(65000),('127.0.0.1',ports['controlPort']));time.sleep(.05)
   source.sendto(b'\x80\x60'+struct.pack('>HII',i,rtp+i*352,0)+encrypted,('127.0.0.1',ports['dataPort']));time.sleep(.008)
  time.sleep(.25)
  result['pcm_bytes']=max(0,media_bytes[0]-65);result['alive']=proc.poll() is None
 except Exception as e:result['error']=str(e)
 finally:
  result['resets']=[e.get('reason') for e in events if e['type']=='flush']
  result['fatal']=sorted({e['message'] for e in events if e['type']=='fatal'})
  if proc:
   result['exit_before_stop']=proc.poll()
   if proc.poll() is None:
    try:command(dict(type='stop'));proc.wait(timeout=3)
    except (OSError,subprocess.TimeoutExpired):proc.kill();proc.wait()
   result['stderr']=proc.stderr.read()[-2000:]
  done.set()
  for s in sockets:s.close()
 return result
try:
 subprocess.run([a.openssl,'genpkey','-algorithm','ED25519','-out',str(key)],check=True,capture_output=True);protect_fixture(key)
 results=[run_case(2,352),run_case(2,None),run_case(2,65538),run_case(1,352,True),run_case(2,'352'),run_case(2,8193),run_case(2,352,control_port=65537)]
 for r in results:
  invalid=r['invalid_control_port'] or isinstance(r['spf'],str) or (r['spf'] is not None and r['spf']>8192)
  r['passed']=not r.get('error') and not r['fatal'] and not r['resets'] and r['exit_before_stop'] is None and (r.get('setup_status')==400 if invalid else r.get('setup_status')==200 and r.get('alive') and r.get('pcm_bytes',0)>0 and r.get('source_frame_count')==352)
 report={'worker_sha256':hashlib.sha256(Path(a.worker).read_bytes()).hexdigest(),'results':results,'passed':all(r['passed'] for r in results),'scope':'synthetic authenticated encrypted audio; not Apple interoperability'}
 report_path.write_text(json.dumps(report,indent=2));print(json.dumps(report,indent=2))
finally:
 for f in state.iterdir():f.unlink()
 state.rmdir()

raise SystemExit(0 if report['passed'] else 1)
