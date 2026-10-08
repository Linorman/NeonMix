#!/usr/bin/env python3
"""Native package runtime and isolated UI maintenance probe. Run through tools/dev.ps1.
No installer execution, firewall change or existing user data access.
"""
import json,os,pathlib,subprocess,time,ctypes,shutil
root=pathlib.Path(__file__).resolve().parents[1];pkg=root/'artifacts/installers/NeonMix';out=root/'artifacts/airplay-reliability';out.mkdir(parents=True,exist_ok=True)
env=os.environ.copy();env['PATH']=str(pkg/'bin')+os.pathsep+env['PATH'];env['GST_PLUGIN_SYSTEM_PATH_1_0']=str(pkg/'plugins');env['GST_PLUGIN_PATH_1_0']='';env['GST_REGISTRY_FORK']='no';env['GST_REGISTRY']=str(root/'.local/tmp/reliability-package-registry.bin')
state=root/'.local/tmp/nonexistent-state';report={}
r=subprocess.run([str(pkg/'bin/neonmix-hub.exe'),'runtime'],env=env,capture_output=True,text=True);assert r.returncode==0 and 'opusenc' in r.stdout;report['packaged_hub_runtime']=True
r=subprocess.run([str(pkg/'bin/neonmix-background.exe'),'--shutdown-installation','--installation-root',str(pkg),'--state-dir',str(state)],capture_output=True,text=True);assert r.returncode==0,r.stderr;report['idle_maintenance']=True
r=subprocess.run([str(pkg/'bin/neonmix-background.exe'),'--shutdown','--state-dir',str(state)],capture_output=True,text=True);assert r.returncode!=0 and not state.exists();report['shutdown_does_not_start_owner']=True
processes=[];states=[]
try:
 for binary,name in [(pkg/'bin/neonmix-desktop.exe','target'),(root/'target/release/neonmix-desktop.exe','other')]:
  folder=root/'.local/tmp'/('reliability-ui-'+name);states.append(folder)
  log=(out/('gui-'+name+'.log')).open('w')
  process=subprocess.Popen([str(binary),'--preview-page','about','--state-dir',str(folder)],env=env,stdout=log,stderr=log);processes.append((process,log))
 user=ctypes.WinDLL('user32');from ctypes import wintypes
 callback_type=ctypes.WINFUNCTYPE(wintypes.BOOL,wintypes.HWND,wintypes.LPARAM)
 def has_window(pid):
  found=[]
  @callback_type
  def visitor(window,_):
   owner=wintypes.DWORD();user.GetWindowThreadProcessId(window,ctypes.byref(owner))
   if owner.value==pid:found.append(window)
   return True
  user.EnumWindows(visitor,0);return bool(found)
 deadline=time.monotonic()+20
 while time.monotonic()<deadline and not all(has_window(p.pid) for p,_ in processes):time.sleep(.1)
 report['owned_windows_ready']=all(has_window(p.pid) for p,_ in processes)
 time.sleep(1)
 if report['owned_windows_ready'] and all(p.poll() is None for p,_ in processes):
  r=subprocess.run([str(pkg/'bin/neonmix-background.exe'),'--shutdown-installation','--installation-root',str(pkg),'--state-dir',str(state)],capture_output=True,text=True);assert r.returncode==0,r.stderr
  assert processes[0][0].wait(timeout=5)==0 and processes[1][0].poll() is None
  report['target_ui_exit']=True;report['other_directory_ui_survived']=True
 else:report['ui_maintenance']='unavailable_native_window'
finally:
 for process,log in processes:
  if process.poll() is None:process.terminate();process.wait(timeout=5)
  log.close()
 for folder in states:
  if folder.exists():shutil.rmtree(folder)
(out/'package-smoke.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8');print(json.dumps(report,indent=2))
