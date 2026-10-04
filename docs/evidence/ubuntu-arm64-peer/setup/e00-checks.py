import json,subprocess,time,pathlib
root=pathlib.Path('/home/parallels/NeonMix');out=root/'artifacts/e00-checks';out.mkdir(exist_ok=True)
commands={
'format':['cargo','fmt','-p','neonmix-core','-p','neonmix-io','-p','neonmix-linux','-p','neonmix-audio','-p','neonmix-desktop','--','--check'],
'core':['cargo','test','--release','-p','neonmix-core','--lib','--test','audio_contract','--test','realtime','--offline','--locked'],
'io':['cargo','test','--release','-p','neonmix-io','--test','pipewire_buffer','--offline','--locked'],
'audio':['cargo','test','--release','-p','neonmix-audio','--bin','neonmix-audio','--test','cli','--offline','--locked'],
'desktop':['cargo','test','--release','-p','neonmix-desktop','--offline','--locked'],
'clippy':['cargo','clippy','--release','-p','neonmix-audio','-p','neonmix-desktop','--offline','--locked','--','-D','warnings'],
'simulate':[str(root/'target/release/neonmix-audio'),'simulate','--input-rate','44100','--output-rate','48000','--seconds','2']}
results={}
for name,cmd in commands.items():
 start=time.monotonic()
 with (out/(name+'.log')).open('w') as log:p=subprocess.run(cmd,cwd=root,stdout=log,stderr=subprocess.STDOUT)
 results[name]={'command':cmd,'exit_code':p.returncode,'seconds':round(time.monotonic()-start,3)}
 (out/'result.json').write_text(json.dumps(results,indent=2)+'\n')
 print(name,p.returncode,flush=True)
 if p.returncode:break
raise SystemExit(0 if len(results)==len(commands) and all(r['exit_code']==0 for r in results.values()) else 1)
