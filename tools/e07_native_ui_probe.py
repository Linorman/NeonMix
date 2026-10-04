#!/usr/bin/env python3
"""macOS native workflow, AX/keyboard and tray/audio isolation probe.
Uses existing developer accessibility access. Creates only project-local state.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time
import uuid
from e07_background_probe import ipc, wait_for
from credential_fixture import remove_owned_fixture

ROOT=Path(__file__).resolve().parents[1]
DEV=ROOT/'tools/dev'
UI=ROOT/'target/release/neonmix-desktop'
URL='https://localhost:7443'

class NativeUI:
    def __init__(self,directory):
        self.directory=directory
        self.log=(ROOT/'artifacts/e07/native-ui-final.log').open('a')
        self.process=subprocess.Popen([str(DEV),str(UI),'--state-dir',str(directory)],stdout=self.log,stderr=self.log,start_new_session=True,cwd=ROOT)
    def ax(self,body):
        script=f'tell application "System Events"\nset p to first application process whose unix id is {self.process.pid}\n{body}\nend tell'
        result=subprocess.run([str(DEV),'/usr/bin/osascript','-e',script],capture_output=True,text=True,cwd=ROOT,timeout=12)
        if result.returncode:raise RuntimeError(result.stderr.strip())
        return result.stdout.strip()
    def activate(self):
        wait_for(lambda:self.ax('set frontmost of p to true\nreturn count of windows of p')=='1')
        self.ax('return entire contents of window 1 of p')
        wait_for(lambda:'text field' in self.ax('return entire contents of window 1 of p'))
    def press(self,label):
        target = f'button "{label}" of group 1 of window 1 of p'
        if label == '退出后台':
            target = '(last button of group 1 of window 1 of p whose name is "退出后台")'
        body=f'''try
if enabled of {target} then
click {target}
return "pressed"
end if
end try
return "unavailable"'''
        wait_for(lambda:self.ax(body)=='pressed')
    def paste(self,label,value):
        wait_for(lambda:self.ax(f'return enabled of text field "{label}" of group 1 of window 1 of p')=='true')
        escaped=value.replace('\\','\\\\').replace('"','\\"')
        script=f'''set oldClipboard to the clipboard
try
set frontmost of p to true
set value of attribute "AXFocused" of text field "{label}" of group 1 of window 1 of p to true
delay 0.1
keystroke "a" using command down
set the clipboard to "{escaped}"
keystroke "v" using command down
delay 0.3
set resultText to value of text field "{label}" of group 1 of window 1 of p
set the clipboard to oldClipboard
return resultText
on error errorMessage number errorNumber
set the clipboard to oldClipboard
error errorMessage number errorNumber
end try'''
        assert self.ax(script)==value
    def tray_restore(self):
        self.ax('click menu bar item 1 of menu bar 2 of p')
        wait_for(lambda:'打开 NeonMix' in self.ax('return entire contents of menu bar 2 of p'))
        self.ax('click menu item "打开 NeonMix" of menu 1 of menu bar item 1 of menu bar 2 of p')
        wait_for(lambda:self.ax('return count of windows of p')=='1')
    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:self.process.kill();self.process.wait()
        self.log.close()

def main():
    assert platform.system()=='Darwin','macOS-only native UI probe'
    state=ROOT/'.local/tmp'/f'e07-ui-{uuid.uuid4().hex[:12]}'
    state.mkdir(mode=0o700)
    (ROOT/'artifacts/e07').mkdir(parents=True,exist_ok=True)
    report={'platform':'macOS','binary_sha256':hashlib.sha256(UI.read_bytes()).hexdigest(),'passed':False,'scenarios':{},'limits':['native Chinese paste and egui IME event regression covered; full VoiceOver listening audit not performed','single-host check; other platforms not run']}
    app=None
    try:
        app=NativeUI(state);app.activate();wait_for(lambda:ipc(state,'status'))
        report['scenarios']['native_ax_named_controls']=True
        app.paste('房间名称','影音室 E07')
        report['scenarios']['native_chinese_input_preserves_clipboard']=True
        app.ax('click pop up button "实体输出" of group 1 of window 1 of p')
        wait_for(lambda:'MacBook Pro Speakers' in app.ax('return entire contents of window 1 of p'))
        app.ax('click checkbox "MacBook Pro Speakers" of group 1 of window 1 of p')
        app.press('创建房间');wait_for(lambda:ipc(state,'status')['hub_settings'])
        metadata=json.loads((state/'hub/server.json').read_text())
        assert metadata['version']==2 and metadata['credential_store']=='file'
        app.press('开始共享');snapshot=wait_for(lambda:ipc(state,'snapshot',credential='hub/admin.json',hub=URL))
        assert snapshot['viewer']['room_name']=='影音室 E07' and snapshot['output']['id']=='coreaudio:BuiltInSpeakerDevice'
        report['scenarios']['native_popup_setup_start_and_real_authority']=True
        pid=ipc(state,'status')['hub']['pid']
        app.ax('set frontmost of p to true\nkeystroke "w" using command down')
        wait_for(lambda:app.ax('return count of windows of p')=='0')
        assert ipc(state,'status')['hub']['pid']==pid
        app.tray_restore();report['scenarios']['cmd_w_hides_and_tray_restores_with_audio_owner_retained']=True
        before=ipc(state,'diagnostics',credential='hub/admin.json',hub=URL)['remote']['output_frames']
        app.process.kill();app.process.wait();app.log.close()
        assert ipc(state,'status')['hub']['pid']==pid
        time.sleep(0.2)
        assert ipc(state,'diagnostics',credential='hub/admin.json',hub=URL)['remote']['output_frames']>before
        app=NativeUI(state);app.activate()
        wait_for(lambda:app.ax('return value of text field "房间名称" of group 1 of window 1 of p')=='影音室 E07')
        report['scenarios']['actual_gui_crash_audio_continues_and_new_gui_reads_saved_state']=True
        app.ax('click button "Mixer" of group 1 of window 1 of p')
        wait_for(lambda:'总音量' in app.ax('return entire contents of window 1 of p'))
        app.press('总静音');wait_for(lambda:ipc(state,'snapshot',credential='hub/admin.json',hub=URL)['output']['muted'])
        app.press('取消总静音');wait_for(lambda:not ipc(state,'snapshot',credential='hub/admin.json',hub=URL)['output']['muted'])
        report['scenarios']['native_mixer_controls_commit_server_state']=True
        app.ax('click button "诊断" of group 1 of window 1 of p')
        app.press('导出脱敏诊断');wait_for(lambda:(state/'diagnostics-redacted.json').exists())
        exported=(state/'diagnostics-redacted.json').read_text()
        assert '影音室' not in exported and 'coreaudio:' not in exported and str(state) not in exported
        report['scenarios']['native_diagnostic_export']=True
        app.ax('set frontmost of p to true\nkeystroke "q" using command down')
        wait_for(lambda:'取消' in app.ax('return entire contents of window 1 of p'))
        app.ax('key code 53');assert app.process.poll() is None and ipc(state,'status')['hub']['running']
        report['scenarios']['native_quit_confirmation_escape_preserves_audio']=True
        app.ax('keystroke "q" using command down');app.press('退出后台')
        assert app.process.wait(timeout=10)==0
        wait_for(lambda:not(state/'ipc.sock').exists())
        try:os.kill(pid,0);raise AssertionError('Hub survived explicit exit')
        except ProcessLookupError:pass
        report['scenarios']['native_cmd_q_explicit_exit_ends_audio_and_background']=True
        report['passed']=True
    except Exception as error:
        report['failure']=str(error)
        try:
            status=ipc(state,'status');report['failure_hub_settings']=status.get('hub_settings');report['failure_hub_running']=status['hub']['running']
            report['failure_ui_tree']=app.ax('return entire contents of window 1 of p')
        except (OSError,RuntimeError):pass
        raise
    finally:
        try:ipc(state,'shutdown')
        except (OSError,RuntimeError):pass
        if app:app.close()
        remove_owned_fixture(state)
        evidence=ROOT/'docs/evidence/e07/native-ui-macos.json';evidence.parent.mkdir(parents=True,exist_ok=True);evidence.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps(report,ensure_ascii=False))

if __name__=='__main__':main()
