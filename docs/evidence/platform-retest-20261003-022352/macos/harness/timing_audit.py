from pathlib import Path
import json
O=Path('/Volumes/projects-mac/NeonMix/artifacts/platform-retest-20261003-022352/macos');audit={}
for name in ['four-airplay-uncapped','four-airplay-contention','four-airplay-contention-cpu-retry','four-airplay-contention-20ms','four-airplay-low-load','mix-2plus2','mix-3plus1','mix-1plus3','mix-0plus4','mix-control','management']:
 d=json.loads((O/(name+'.json')).read_text());windows=[]
 if 'steady_continuity' in d:windows.append(('steady',d['steady_continuity']))
 for i,c in enumerate(d.get('isolation_checks',[])):
  for key in ['baseline','fault_interval','recovery_interval']:
   if key in c:windows.append((f'fault_{i}_{key}',c[key]))
 if 'mix_control_reconnect' in d:windows.append(('mix_control_reconnect',d['mix_control_reconnect']['continuity']))
 data=[dict(name=n,**w) for n,w in windows]
 global_nonzero=[x['name'] for x in data if x.get('global_timed_late_frames_delta',0)!=0]
 lane_nonzero=[dict(window=x['name'],kind=l.get('kind'),lane=l.get('lane'),source_index=l.get('source_index'),underrun_delta=l.get('underrun_frames_delta',0),late_delta=l.get('late_packets_delta',0)) for x in data for l in x.get('lanes',[]) if l.get('underrun_frames_delta',0)!=0 or l.get('late_packets_delta',0)!=0]
 audit[name]=dict(functional_probe_passed=d['passed'],windows=len(data),global_late_nonzero_windows=global_nonzero,lane_nonzero_windows=lane_nonzero,max_global_late_delta=max([x.get('global_timed_late_frames_delta',0) for x in data],default=None),raw_windows=data,completed_recoveries=sum('recovery_interval' in c for c in d.get('isolation_checks',[])),status_reads=d.get('status_reads'))
 print(name,'global_nonzero',len(global_nonzero),'lane_nonzero',len(lane_nonzero),'maxglobal',audit[name]['max_global_late_delta'])
for name in ['single-pcm','single-alac']:
 d=json.loads((O/(name+'.json')).read_text());audit[name]=dict(functional_probe_passed=d['passed'],timing=d['timing'],pause_resume_cycles=d['pause_resume_cycles'])
(O/'timing-audit.json').write_text(json.dumps(audit,indent=2,ensure_ascii=False)+'\n')
