"""Actual GUI fixture; isolate listen port and preserve binary rpath layout."""
from pathlib import Path
import sys,subprocess,json,shutil,time,hashlib,os,uuid,socket,traceback
R=Path('/Volumes/projects-mac/NeonMix/.local/tm3')
O=Path('/Volumes/projects-mac/NeonMix/artifacts/platform-serial-retest-20261003-1430/macos')
sys.path.insert(0,str(R/'tools'))
from e07_native_ui_probe import NativeUI
from e07_background_probe import ipc,wait_for,prepare_isolated_binaries
from credential_fixture import remove_owned_fixture
from macos_ui_visual_probe import scroll_window
DEV=R/'tools/dev'
sock=socket.socket();sock.bind(('127.0.0.1',0));port=sock.getsockname()[1];sock.close()
base=R/'.local/tmp'/('ui4-'+uuid.uuid4().hex[:8]);state=base/'s';state.mkdir(mode=0o700,parents=True)
binary=prepare_isolated_binaries(base,R/'target/release',port)
for n in ('neonmix-desktop','neonmix-airplay-worker'):shutil.copy2(R/'target/release'/n,binary/n)
URL=f'https://localhost:{port}'
report={'platform':'macOS','passed':False,'scope':'formal non-preview isolated desktop, actual AX interactions and real CoreAudio/Hub authority','scenarios':{},'limits':['not VoiceOver listening or genuine IME composition','explicit BlackHole digital output; no physical listening'],'binary_sha256':{n:hashlib.sha256((binary/n).read_bytes()).hexdigest() for n in ('neonmix-desktop','neonmix-background','neonmix-hub-real')},'listen_port':port}
class Window(NativeUI):
 def __init__(self):
  self.log=(O/'formal-ui-process.log').open('a')
  self.process=subprocess.Popen([str(DEV),str(binary/'neonmix-desktop'),'--state-dir',str(state)],cwd=R,stdout=self.log,stderr=self.log)
 def ax(self,body):
  last=None
  for _ in range(5):
   try:return super().ax(body)
   except RuntimeError as e:
    last=e
    if self.process.poll() is not None:raise RuntimeError(f'owned GUI exited: {self.process.returncode}') from e
    time.sleep(.2)
  raise last
 def press(self,label):
  if ('checkbox '+label+' of group') in self.ax('return entire contents of window 1 of p'):
   self.ax(f'click checkbox "{label}" of group 1 of window 1 of p');time.sleep(.3)
  else:super().press(label)
 def nav(self,label):
  self.ax(f'click checkbox "{label}" of group 1 of window 1 of p')
  time.sleep(.4)
 def screenshot(self,name):
  bounds=self.ax('set xy to position of window 1 of p\nset wh to size of window 1 of p\nreturn (item 1 of xy as text) & "," & (item 2 of xy as text) & "," & (item 1 of wh as text) & "," & (item 2 of wh as text)')
  p=subprocess.run([str(DEV),'/usr/sbin/screencapture','-x','-R'+bounds,str(O/(name+'.png'))],cwd=R,capture_output=True,text=True)
  if p.returncode:report.setdefault('capture_errors',{})[name]=p.stderr.strip()
  (O/(name+'-ax.txt')).write_text(self.ax('return entire contents of window 1 of p'))
 def endpoint(self):
  self.nav('诊断')
  tree=self.ax('return entire contents of window 1 of p')
  if '连接地址（故障排查）' not in tree:self.press('连接与进程')
  tree=self.ax('return entire contents of window 1 of p')
  if 'HTTPS 地址（可留空）' not in tree:self.press('连接地址（故障排查）')
  self.paste('HTTPS 地址（可留空）',URL)
  self.nav('Hub 设置')
app=None;pid=None
try:
 app=Window();app.activate();wait_for(lambda:ipc(state,'status'))
 report['scenarios']['formal_window_owned_background_and_ax_controls']=True
 app.endpoint()
 app.paste('房间名称','本轮测试影音室 Oct3')
 for label in ('Sender','Mixer','设备管理','诊断','Hub 设置'):app.nav(label)
 assert app.ax('return value of text field "房间名称" of group 1 of window 1 of p')=='本轮测试影音室 Oct3'
 report['scenarios']['five_page_navigation_chinese_clipboard_paste_and_draft_persistence']=True
 app.ax('click pop up button "实体输出" of group 1 of window 1 of p')
 wait_for(lambda:'MacBook Pro Speakers' in app.ax('return entire contents of window 1 of p'))
 assert 'checkbox BlackHole 2ch' not in app.ax('return entire contents of window 1 of p')
 app.ax('click checkbox "MacBook Pro Speakers" of group 1 of window 1 of p')
 app.press('创建房间');settings=wait_for(lambda:ipc(state,'status')['hub_settings'])
 assert settings['output']=='coreaudio:BuiltInSpeakerDevice'
 report['scenarios']['native_physical_output_popup_and_create_room_without_audio_start']=True
 report['fixture_adaptation']='GUI physical selector intentionally omits BlackHole; after native create, owned background hub_settings selects BlackHole before starting any audio; reopened UI loads that authoritative fixture.'
 ipc(state,'hub_settings',settings={'name':'本轮测试影音室 Oct3','output':'coreaudio:BlackHole2ch_UID'})
 app.close();app=Window();app.activate();app.endpoint()
 app.press('开始共享')
 snap=wait_for(lambda:ipc(state,'snapshot',credential='hub/admin.json',hub=URL))
 assert snap['viewer']['role']=='admin' and snap['output']['id']=='coreaudio:BlackHole2ch_UID'
 pid=ipc(state,'status')['hub']['pid'];report['scenarios']['native_popup_create_start_blackhole_and_authenticated_authority']=True
 app.screenshot('formal-room')
 app.nav('Mixer');wait_for(lambda:'总音量' in app.ax('return entire contents of window 1 of p'))
 app.press('总静音');wait_for(lambda:ipc(state,'snapshot',credential='hub/admin.json',hub=URL)['output']['muted'])
 app.press('取消总静音');wait_for(lambda:not ipc(state,'snapshot',credential='hub/admin.json',hub=URL)['output']['muted'])
 report['scenarios']['native_mixer_mute_commits_and_unmutes_server']=True
 app.screenshot('formal-mixer')
 report['shortcut_checks']=[]
 for width,height in [(1100,760),(600,440)]:
  app.ax(f'set size of window 1 of p to {{{width}, {height+32}}}')
  time.sleep(.4)
  app.ax('set frontmost of p to true\nkeystroke "k" using command down')
  wait_for(lambda:'前往 Hub 设置' in app.ax('return entire contents of window 1 of p'))
  app.screenshot('formal-palette-'+str(width))
  visible=app.ax('set e to static text "关闭" of group 1 of window 1 of p\nset ep to position of e\nset es to size of e\nset wp to position of window 1 of p\nset ws to size of window 1 of p\nreturn (item 2 of ep) + (item 2 of es) <= (item 2 of wp) + (item 2 of ws) - 8')
  assert visible=='true','formal palette footer extends beyond the window'
  actual=app.ax('set ws to size of window 1 of p\nreturn (item 1 of ws as text) & "," & (item 2 of ws as text)')
  app.ax('key code 53')
  app.ax('keystroke "2" using command down')
  wait_for(lambda:'设备名称' in app.ax('return entire contents of window 1 of p'))
  app.screenshot('formal-sender-'+str(width))
  assert actual==f'{width},{height+32}',f'actual native window size {actual}'
  report['shortcut_checks'].append(dict(requested_content_width=width,requested_content_height=height,actual_window_size=actual,native_titlebar_height=32,command_k_opens_palette=True,escape_closes_palette=True,command_2_navigates=True,palette_footer_within_window=True))
  app.nav('Mixer')
 report['scenarios']['formal_normal_and_minimum_window_command_shortcuts']=True
 app.ax('set size of window 1 of p to {1100,760}')
 app.nav('诊断');app.press('导出脱敏诊断');wait_for(lambda:(state/'diagnostics-redacted.json').exists())
 exported=(state/'diagnostics-redacted.json').read_text()
 assert '本轮测试影音室' not in exported and 'coreaudio:' not in exported and str(state) not in exported
 report['scenarios']['native_redacted_diagnostic_export']=True
 app.ax('set frontmost of p to true\nkeystroke "w" using command down');wait_for(lambda:app.ax('return count of windows of p')=='0')
 assert ipc(state,'status')['hub']['pid']==pid
 app.tray_restore();report['scenarios']['command_w_hides_tray_restores_hub_pid_retained']=True
 before=ipc(state,'diagnostics',credential='hub/admin.json',hub=URL)['remote']['output_frames']
 app.process.kill();app.process.wait();app.log.close();time.sleep(.3)
 assert ipc(state,'status')['hub']['pid']==pid and ipc(state,'diagnostics',credential='hub/admin.json',hub=URL)['remote']['output_frames']>before
 app=Window();app.activate();app.endpoint()
 wait_for(lambda:app.ax('return value of text field "房间名称" of group 1 of window 1 of p')=='本轮测试影音室 Oct3')
 report['scenarios']['gui_crash_audio_continues_and_reopen_loads_saved_room']=True
 app.ax('set frontmost of p to true\nkeystroke "q" using command down');wait_for(lambda:'取消' in app.ax('return entire contents of window 1 of p'))
 app.ax('key code 53');assert app.process.poll() is None and ipc(state,'status')['hub']['running']
 report['scenarios']['command_q_confirmation_escape_keeps_audio']=True
 app.ax('keystroke "q" using command down');app.press('退出后台')
 assert app.process.wait(timeout=10)==0
 wait_for(lambda:not(state/'ipc.sock').exists())
 try:os.kill(pid,0);raise AssertionError('Hub survived confirmed quit')
 except ProcessLookupError:pass
 report['scenarios']['confirmed_quit_ends_gui_background_and_hub']=True
 report['passed']=True
except Exception as e:
 report['failure_type']=type(e).__name__;report['failure']=str(e);report['failure_location']=[{'file':Path(x.filename).name,'line':x.lineno,'function':x.name} for x in traceback.extract_tb(e.__traceback__)]
 if app:
  try:app.screenshot('formal-failure')
  except Exception:pass
finally:
 try:ipc(state,'shutdown')
 except Exception:pass
 if app:app.close()
 remove_owned_fixture(base)
 report['fixture_removed']=not base.exists()
 (O/'formal-ui.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
 print(json.dumps(report,ensure_ascii=False))
raise SystemExit(0 if report['passed'] else 1)
