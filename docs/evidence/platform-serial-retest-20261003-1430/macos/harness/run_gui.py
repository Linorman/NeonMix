from pathlib import Path
import json,time,subprocess,os,signal
ROOT=Path('/Volumes/projects-mac/NeonMix/.local/tm3')
OUT=Path('/Volumes/projects-mac/NeonMix/artifacts/platform-serial-retest-20261003-1430/macos')
cmd=[str(ROOT/'tools/dev'),'python3',str(OUT/'harness/formal_ui.py')]
start=time.time()
with (OUT/'formal-ui.log').open('w') as f:
 p=subprocess.Popen(cmd,cwd=ROOT,stdout=f,stderr=subprocess.STDOUT,start_new_session=True)
 try:code=p.wait(timeout=450)
 except subprocess.TimeoutExpired:
  os.killpg(p.pid,signal.SIGINT)
  try:code=p.wait(timeout=15)
  except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);code=p.wait()
row=dict(name='formal-ui',command=cmd,exit_code=code,start_epoch=start,end_epoch=time.time(),seconds=round(time.time()-start,3))
(OUT/'gui-checks.json').write_text(json.dumps([row],indent=2)+'\n');print(json.dumps(row),flush=True)
raise SystemExit(code)
