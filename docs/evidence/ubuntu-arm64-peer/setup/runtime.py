import subprocess,pathlib,json,re
root=pathlib.Path('/home/parallels/NeonMix');out=root/'artifacts';meta=subprocess.check_output(['pw-metadata','-n','settings'],text=True);(out/'pipewire-settings-before.log').write_text(meta)
match=re.search(r"key:'clock.allowed-rates' value:'([^']+)'",meta);assert match
original=match.group(1);report={'original_allowed_rates':original}
try:
 subprocess.run(['pw-metadata','-n','settings','0','clock.allowed-rates','[ 44100 48000 96000 ]'],check=True,capture_output=True)
 with (out/'runtime.log').open('w') as log:p=subprocess.run(['python3','tools/linux_runtime_probe.py'],cwd=root,stdout=log,stderr=subprocess.STDOUT,timeout=180)
 report['exit_code']=p.returncode
finally:
 restore=subprocess.run(['pw-metadata','-n','settings','0','clock.allowed-rates',original],capture_output=True,text=True)
 report['restore_exit_code']=restore.returncode
 (out/'pipewire-settings-after.log').write_text(subprocess.check_output(['pw-metadata','-n','settings'],text=True))
 (out/'runtime-session.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report))
raise SystemExit(report.get('exit_code',1))
