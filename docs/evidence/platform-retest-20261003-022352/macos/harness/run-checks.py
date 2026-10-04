from pathlib import Path
import subprocess,time,json,sys,os,statistics,signal,shutil
os.environ["CARGO_BUILD_JOBS"]="2"
R=Path('/Volumes/projects-mac/NeonMix/.local/t04m'); O=Path('/Volumes/projects-mac/NeonMix/artifacts/platform-retest-20261003-022352/macos')
REPORTS=R/'artifacts/platform-retest-20261003-022352/macos';REPORTS.mkdir(parents=True,exist_ok=True)
def multi(name,*args):return name,['python3','tools/airplay_multi_source_probe.py','--output','coreaudio:BlackHole2ch_UID','--report',str(REPORTS/(name+'.json')),*args]
quality=[('workspace-tests',['cargo','test','--workspace','--locked','--','--test-threads=1']),('strict-clippy',['cargo','clippy','--workspace','--all-targets','--locked','--','-D','warnings']),('workspace-fmt',['cargo','fmt','--all','--','--check']),('worker-protocol',['python3','apps/airplay-worker/probe.py']),('worker-pairing',['python3','apps/airplay-worker/pairing_probe.py']),('worker-decoder',['.local/airplay/build/neonmix-airplay-audio-probe']),('worker-identity',['python3','tools/airplay_identity_probe.py','--worker','.local/airplay/build/neonmix-airplay-worker','--report',str(REPORTS/'worker-identity.json')])]
runtime=[multi('four-airplay-contention','--sources','4','--fault-cycles','3','--status-readers','4','--steady-seconds','30'),multi('mix-control','--scenario','mix-control','--sources','2','--status-readers','4'),multi('management','--scenario','management','--sources','2'),multi('mix-2plus2','--sources','2','--native-sources','2','--fault-cycles','2'),multi('mix-3plus1','--sources','3','--native-sources','1'),multi('mix-1plus3','--sources','1','--native-sources','3'),multi('mix-0plus4','--sources','0','--native-sources','4','--steady-seconds','60')]
for codec in ['pcm','alac']:runtime.append(('single-'+codec,['python3','tools/airplay_mixer_probe.py','--profile','release','--output','coreaudio:BlackHole2ch_UID','--codec',codec,'--pause-seconds','1','--pause-cycles','2','--resume-reset','setup','--report',str(REPORTS/('single-'+codec+'.json'))]))
runtime.append(('background',['python3','tools/e07_background_probe.py','--release','--output','coreaudio:BlackHole2ch_UID','--evidence',str(REPORTS/'background.json')]))
def cpu_rows():
 try:return [json.loads(x) for x in (O/'cpu-monitor.jsonl').read_text().splitlines()]
 except Exception:return []
def idle():
 while (O/'pause-runtime').exists():time.sleep(1)
def cpu_summary(start,end):
 r=[x['util'] for x in cpu_rows() if start<=x['epoch']<=end];above=[x>80 for x in r]
 return {'samples':len(r),'mean':round(statistics.mean(r),3) if r else None,'p95':sorted(r)[min(len(r)-1,int(.95*len(r)))] if r else None,'max':max(r) if r else None,'minimum_headroom':round(100-max(r),3) if r else None,'above80':sum(above),'near_full':any(x>=95 for x in r),'sustained_above80':any(a and b for a,b in zip(above,above[1:])),'condition_met':bool(r) and not any(x>=95 for x in r) and not any(a and b for a,b in zip(above,above[1:]))}
bounded=multi('four-airplay-contention-20ms','--sources','4','--fault-cycles','3','--status-readers','4','--steady-seconds','30');bounded[1][1]=str(O/'harness/four-bounded.py')
checks=[multi('four-airplay-uncapped','--sources','4','--fault-cycles','3','--status-readers','4','--steady-seconds','30')] if sys.argv[1]=='runtime-uncapped' else [multi('four-airplay-low-load','--sources','4','--native-sources','0','--status-readers','0','--fault-cycles','1','--steady-seconds','20')] if sys.argv[1]=='runtime-low-load' else [bounded] if sys.argv[1]=='runtime-bounded' else [multi('four-airplay-contention-cpu-retry','--sources','4','--fault-cycles','3','--status-readers','4','--steady-seconds','30')] if sys.argv[1]=='runtime-cpu-retry' else [('worker-protocol-retry',['python3',str(O/'harness/protocol-diagnostic.py')]),('worker-pairing-retry',['python3','apps/airplay-worker/pairing_probe.py']),('worker-identity-retry',['python3','tools/airplay_identity_probe.py','--worker','.local/airplay/build/neonmix-airplay-worker','--report',str(REPORTS/'worker-identity-retry.json')])] if sys.argv[1]=='worker-retry' else quality if sys.argv[1]=='quality' else [('member-fmt',['python3','-c',"import subprocess,json;d=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1','--locked']));cmd=['cargo','fmt'];[cmd.extend(['-p',p['name']]) for p in d['packages'] if p['id'] in d['workspace_members']];raise SystemExit(subprocess.run(cmd+['--','--check']).returncode)"])] if sys.argv[1]=='member' else [('formal-ui',['python3',str(O/'harness/formal_ui.py')])] if sys.argv[1]=='gui' else runtime
res=[]
for n,args in checks:
 idle()
 epoch=time.time();start=time.monotonic();cmd=[str(R/'tools/dev'),*args]
 with (O/(n+'.log')).open('w') as f:
  p=subprocess.Popen(cmd,cwd=R,stdout=f,stderr=subprocess.STDOUT,start_new_session=True);cpu_aborted=False
  while p.poll() is None:
   time.sleep(.5)
   stat=cpu_summary(epoch,time.time())
   if time.monotonic()-start>900:
    os.killpg(p.pid,signal.SIGINT)
    try:p.wait(timeout=30)
    except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
    break
  code=p.returncode
  while time.monotonic()-start<1.1:time.sleep(.1)

 item=dict(name=n,command=cmd,exit_code=code,seconds=round(time.monotonic()-start,3),start_epoch=epoch,end_epoch=time.time(),cpu=cpu_summary(epoch,time.time()));
 item['cpu']['condition_met']=None;item['cpu_policy']='observation only; no percentage admission/abort threshold, affinity or hard cap; own compile/other stress paused'
 res.append(item);(O/(sys.argv[1]+'-checks.json')).write_text(json.dumps(res,indent=2));print(json.dumps(item),flush=True)
 if (REPORTS/(n+'.json')).exists():shutil.copy2(REPORTS/(n+'.json'),O/(n+'.json'))
 if code:print((O/(n+'.log')).read_text()[-1800:],flush=True)

raise SystemExit(0 if all(x['exit_code']==0 for x in res) else 1)
