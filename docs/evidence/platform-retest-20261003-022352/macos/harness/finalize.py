from pathlib import Path
import json,hashlib,re,shutil,statistics,time,subprocess
R=Path('/Volumes/projects-mac/NeonMix');W=R/'.local/t04m';O=R/'artifacts/platform-retest-20261003-022352/macos';D=R/'docs/evidence/platform-retest-20261003-022352/macos'
manifest=json.loads((O.parent/'source-manifest.json').read_text())
mismatch=[k for k,v in manifest.items() if hashlib.sha256((W/k).read_bytes()).hexdigest()!=v]
binary={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in (O/'bin').iterdir() if p.is_file()}
before=json.loads((O/'binary-provenance.json').read_text())['binary_sha256']
source={'files':len(manifest),'mismatch':mismatch,'unchanged':not mismatch,'immutable_binary_unchanged':before==binary}
(O/'source-after.json').write_text(json.dumps(source,indent=2)+'\n')
summary={'source':source,'binary_sha256':binary,'fixture_output':'coreaudio:BlackHole2ch_UID','checks':{},'runtime':{}}
for section in ['build','quality','worker-retry','runtime','runtime-cpu-retry','runtime-bounded','runtime-low-load','runtime-uncapped','gui','member']:
 p=O/(section+'-checks.json')
 if p.exists():summary['checks'][section]=json.loads(p.read_text())
testlog=O/'workspace-tests.log'
if testlog.exists():
 rows=re.findall(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;',testlog.read_text())
 summary['workspace_tests']={k:sum(int(row[i]) for row in rows) for i,k in enumerate(['passed','failed','ignored'])}
for name in ['four-airplay-uncapped','worker-protocol-retry','worker-pairing-retry','worker-identity-retry','four-airplay-contention-cpu-retry','four-airplay-contention-20ms','four-airplay-low-load','four-airplay-contention','mix-control','management','mix-2plus2','mix-3plus1','mix-1plus3','mix-0plus4','single-pcm','single-alac','background','formal-ui']:
 p=O/(name+'.json')
 if p.exists():
  d=json.loads(p.read_text());summary['runtime'][name]={k:d[k] for k in ['passed','failure','failure_phase','failure_type','failure_hub_running','protocol_failure','sources','native_sources','fault_cycles','scenarios','observations','checks','fixture_removed','status_reader_stats'] if k in d}
summary['preexisting_processes_alive']={str(pid):subprocess.run(['kill','-0',str(pid)],capture_output=True).returncode==0 for pid in [81271,98624,98625,98648]}
(O/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n')
D.mkdir(parents=True,exist_ok=True)
for p in O.iterdir():
 if p.is_file() and p.suffix in ['.json','.jsonl','.log','.png','.txt']:shutil.copy2(p,D/p.name)
if (O/'harness').is_dir():shutil.copytree(O/'harness',D/'harness',dirs_exist_ok=True)
print(json.dumps(summary,ensure_ascii=False))
