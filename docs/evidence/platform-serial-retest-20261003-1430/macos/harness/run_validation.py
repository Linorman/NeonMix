from pathlib import Path
import json, os, signal, subprocess, sys, time, shutil
ROOT = Path('/Volumes/projects-mac/NeonMix/.local/tm3')
OUT = Path('/Volumes/projects-mac/NeonMix/artifacts/platform-serial-retest-20261003-1430/macos')

REPORTS=ROOT/'artifacts/serial-results'; REPORTS.mkdir(parents=True,exist_ok=True)

def multi(name, *args):
    return name, ['python3', 'tools/airplay_multi_source_probe.py', '--output', 'coreaudio:BlackHole2ch_UID', '--report', str(REPORTS / (name + '.json')), *args]

build = [
    ('member-fmt', ['python3','-c',"import subprocess,json;d=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1','--locked']));cmd=['cargo','fmt'];[cmd.extend(['-p',p['name']]) for p in d['packages'] if p['id'] in d['workspace_members']];raise SystemExit(subprocess.run(cmd+['--','--check']).returncode)"]),
    ('strict-clippy', ['cargo','clippy','--workspace','--all-targets','--locked','--','-D','warnings']),
    ('workspace-tests', ['cargo', 'test', '--workspace', '--locked', '--', '--test-threads=1']),
    ('release', ['cargo','build','--workspace','--release','--locked']),
    ('worker-build', ['python3','tools/prepare_airplay.py']),
    ('worker-probes-build',['cmake','--build','.local/airplay/build','--target','neonmix-airplay-probe-crypto','neonmix-airplay-audio-probe','neonmix-airplay-identity-probe','--parallel','2']),
]
worker = [
 ('worker-protocol',['python3','apps/airplay-worker/probe.py']),
 ('worker-pairing',['python3','apps/airplay-worker/pairing_probe.py']),
 ('worker-identity',['python3','tools/airplay_identity_probe.py','--worker','.local/airplay/build/neonmix-airplay-worker','--report',str(REPORTS/'worker-identity.json')]),
 ('worker-decoder',['.local/airplay/build/neonmix-airplay-audio-probe']),
 ('credential-store',['python3','tools/credential_store_probe.py','--release']),
 ('background',['python3','tools/e07_background_probe.py','--release','--output','coreaudio:BlackHole2ch_UID','--evidence',str(REPORTS/'background.json')]),
]
runtime = [
    multi('mix-control','--scenario','mix-control','--sources','2','--status-readers','4'),
    multi('management','--scenario','management','--sources','2'),
    multi('four-airplay', '--sources', '4', '--status-readers', '4', '--steady-seconds', '30', '--fault-cycles', '3'),
    multi('mix-3plus1', '--sources', '3', '--native-sources', '1', '--steady-seconds', '20'),
    multi('mix-2plus2', '--sources', '2', '--native-sources', '2', '--steady-seconds', '15', '--fault-cycles', '2'),
    multi('mix-1plus3', '--sources', '1', '--native-sources', '3', '--steady-seconds', '15'),
    multi('mix-0plus4', '--sources', '0', '--native-sources', '4', '--steady-seconds', '60'),
]
for codec in ['pcm', 'alac']:
    name = 'single-' + codec
    runtime.append((name, ['python3', 'tools/airplay_mixer_probe.py', '--profile', 'release', '--output', 'coreaudio:BlackHole2ch_UID', '--codec', codec, '--pause-seconds', '1', '--pause-cycles', '2', '--resume-reset', 'setup', '--report', str(REPORTS / (name + '.json'))]))
runtime.append(('single-sync', ['python3', 'tools/airplay_mixer_probe.py', '--profile', 'release', '--output', 'coreaudio:BlackHole2ch_UID', '--playback-mode', 'synchronized', '--report', str(REPORTS / 'single-sync.json')]))
runtime.append(('worker-decoder', ['.local/airplay/build/neonmix-airplay-audio-probe']))
mode = sys.argv[1]
checks = build if mode == 'build' else [worker[-1]] if mode == 'background' else worker if mode == 'worker' else runtime
if mode in ('runtime','worker') and len(sys.argv) > 2:
    checks = [item for item in checks if item[0] in sys.argv[2:]]
results = []
monitor = None
try:
    if mode == 'runtime':
        monitor = subprocess.Popen([sys.executable, str(OUT / 'harness/cpu_monitor.py'), str(OUT / 'cpu.jsonl')])
    for name, command in checks:
        start = time.time()
        with (OUT / (name + '.log')).open('w') as log:
            child = subprocess.Popen([str(ROOT / 'tools/dev'), *command], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True, env=dict(os.environ, CARGO_BUILD_JOBS='2'))
            try:
                code = child.wait(timeout=900)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGINT)
                try: code = child.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    code = child.wait()
        if (REPORTS/(name+'.json')).exists(): shutil.copy2(REPORTS/(name+'.json'),OUT/(name+'.json'))
        row = dict(name=name, command=[str(ROOT / 'tools/dev'),*command], exit_code=code, start_epoch=start, end_epoch=time.time(), seconds=round(time.time()-start, 3))
        results.append(row)
        (OUT / (mode + '-checks.json')).write_text(json.dumps(results, indent=2) + '\n')
        print(json.dumps(row), flush=True)
        if code:
            print((OUT / (name + '.log')).read_text()[-1600:], flush=True)
            if mode == 'build': break
finally:
    if monitor:
        monitor.terminate()
        monitor.wait(timeout=5)
raise SystemExit(0 if all(r['exit_code'] == 0 for r in results) else 1)
