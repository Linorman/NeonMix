from pathlib import Path
import subprocess,time,json,os
os.environ["CARGO_BUILD_JOBS"]="2"
R=Path('/Volumes/projects-mac/NeonMix/.local/t04m'); O=Path('/Volumes/projects-mac/NeonMix/artifacts/platform-retest-20261003-022352/macos')
checks=[('release',['cargo','build','--workspace','--release','--locked']),('worker-build',['python3','-c',"from pathlib import Path; p=Path('tools/prepare_airplay.py');exec(compile(p.read_text().replace(\"'--parallel','8'\",\"'--parallel','2'\"),str(p),'exec'),{'__file__':str(p.resolve()),'__name__':'__main__'})"]),('worker-probes-build',['cmake','--build','.local/airplay/build','--target','neonmix-airplay-probe-crypto','neonmix-airplay-audio-probe','neonmix-airplay-identity-probe','--parallel','2'])]
res=[]
for n,args in checks:
 start=time.monotonic(); cmd=[str(R/'tools/dev'),*args]
 with (O/(n+'.log')).open('w') as f:p=subprocess.run(cmd,cwd=R,stdout=f,stderr=subprocess.STDOUT)
 item=dict(name=n,command=cmd,exit_code=p.returncode,seconds=round(time.monotonic()-start,3));res.append(item);(O/'build-checks.json').write_text(json.dumps(res,indent=2));print(json.dumps(item),flush=True)
 if p.returncode:break
