#!/usr/bin/env python3
"""Exercise Sender pairing/binding and device authorization using native UI."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time
import uuid

from e07_background_probe import ipc, wait_for
from e07_native_ui_probe import NativeUI
from macos_ui_visual_probe import VisualWindow, click_native_button, scroll_window

from credential_fixture import remove_owned_fixture

ROOT = Path(__file__).resolve().parents[1]
DEV = ROOT / "tools/dev"
BIN = ROOT / "target/release"
URL = "https://localhost:7443"


def main():
    global BIN
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary-dir", type=Path, default=BIN)
    args = parser.parse_args()
    BIN = args.binary_dir.resolve()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    base = ROOT / ".local/tmp" / ("sender-ui-" + uuid.uuid4().hex[:12])
    hubdir, senderdir = base / "hub", base / "sender"
    hubdir.mkdir(parents=True, mode=0o700)
    senderdir.mkdir(mode=0o700)
    report = {"passed": False, "binary_sha256": hashlib.sha256((BIN / "neonmix-desktop").read_bytes()).hexdigest(), "scenarios": {}, "limits": ["fields located by accessible names", "single host; no real IME composition or VoiceOver listening"]}
    children, handles, windows = [], [], []

    class Window(NativeUI):
        def __init__(self, directory, name):
            self.log = (out / (name + ".log")).open("w")
            self.process = subprocess.Popen([str(DEV), str(BIN / "neonmix-desktop"), "--state-dir", str(directory)], cwd=ROOT, stdout=self.log, stderr=self.log)
            windows.append(self)
            self.activate()
            time.sleep(1.5)

        def ax(self, body):
            body = body.replace('text field "邀请文本"', 'text field "粘贴 Hub 给出的一次性邀请"')
            return super().ax(body)

        def press(self, label):
            click_native_button(self, label)

        def paste(self, label, value):
            if label == "虚拟输出名称":
                scroll_window(self, -30)
            try:
                super().paste(label, value)
            except AssertionError:
                if label != "邀请文本":
                    raise
                # Native secure fields expose masked AX values. The following
                # pairing request, not a plaintext readback, verifies the paste.
                report["masked_invitation_readback"] = True

        def screenshot(self, name):
            VisualWindow.capture(self, out, name)

    def snapshot():
        return ipc(hubdir, "snapshot", credential="hub/admin.json", hub=URL)

    try:
        log = (out / "hub-background.log").open("w")
        handles.append(log)
        children.append(subprocess.Popen([str(DEV), str(BIN / "neonmix-background"), "--state-dir", str(hubdir)], stdout=log, stderr=log, cwd=ROOT))
        wait_for(lambda: ipc(hubdir, "status"))
        ipc(hubdir, "hub_setup", settings={"name": "本机 UI 验收房间", "output": "coreaudio:BuiltInSpeakerDevice"})
        ipc(hubdir, "hub_start")
        wait_for(snapshot)
        app = Window(senderdir, "sender-ui")
        app.press("Sender")
        wait_for(lambda: "设备名称" in app.ax("return entire contents of window 1 of p"))
        time.sleep(.5)
        app.press("发现局域网房间")
        try:
            wait_for(lambda: "本机 UI 验收房间" in app.ax("return entire contents of window 1 of p"), seconds=15)
            report["scenarios"]["native_discovery_shows_actual_room"] = True
        except RuntimeError:
            report["scenarios"]["native_discovery_shows_actual_room"] = False
            app.screenshot("discovery-no-room")
            report["discovery_ipc_comparison"] = ipc(senderdir, "discover", seconds=3)
        invite = ipc(hubdir, "invite", credential="hub/admin.json", hub=URL, out="invitations/ui.json", seconds=120)
        app.paste("设备名称", "中文 Sender UI 验收")
        app.paste("邀请文本", invite["invitation"])
        app.press("配对房间")
        wait_for(lambda: (senderdir / "profiles/sender.json").exists())
        member = wait_for(lambda: ipc(senderdir, "snapshot", credential="profiles/sender.json", hub=URL))["viewer"]["device_id"]
        report["scenarios"]["native_invitation_paste_pair_commits_member_identity"] = True
        scroll_window(app, -30)
        app.ax('click pop up button "虚拟输出提供者" of group 1 of window 1 of p')
        app.ax('click checkbox "BlackHole（外部提供者）" of group 1 of window 1 of p')
        time.sleep(.3)
        app.ax("key code 53")
        time.sleep(.3)
        app.paste("虚拟输出名称", "NeonMix — 中文 UI 输出")
        app.press("添加虚拟输出")
        binding = wait_for(lambda: ipc(senderdir, "status")["output_binding"])
        original_id = binding["output_id"]
        report["scenarios"]["native_provider_selection_and_binding_add"] = True
        app.paste("虚拟输出名称", "NeonMix — 改名 UI 输出")
        app.press("保存输出名称")
        wait_for(lambda: ipc(senderdir, "status")["output_binding"]["display_name"] == "NeonMix — 改名 UI 输出")
        assert ipc(senderdir, "status")["output_binding"]["output_id"] == original_id
        report["scenarios"]["native_binding_rename_keeps_identity"] = True
        app.press("开始发送")
        wait_for(lambda: ipc(senderdir, "status")["sender"]["running"])
        wait_for(lambda: len(snapshot()["streams"]) == 1)
        app.screenshot("sender-running")
        app.press("停止发送")
        wait_for(lambda: not ipc(senderdir, "status")["sender"]["running"])
        assert ipc(hubdir, "status")["hub"]["running"]
        report["scenarios"]["native_start_stop_sender_retains_hub"] = True
        app.press("禁用输出")
        wait_for(lambda: not ipc(senderdir, "status")["output_binding"]["enabled"])
        app.press("启用输出")
        wait_for(lambda: ipc(senderdir, "status")["output_binding"]["enabled"])
        assert not ipc(senderdir, "status")["sender"]["running"]
        report["scenarios"]["native_disable_enable_does_not_autostart"] = True
        # System Events can canonicalize two unbundled processes with the same
        # name to the wrong AX process. Keep one GUI open; both owners remain.
        app.close()
        admin = Window(hubdir, "admin-ui")
        admin.press("设备管理")
        admin.paste("搜索设备", "中文 Sender UI 验收")
        admin.press("断开设备")
        wait_for(lambda: not snapshot()["devices"][member]["playback_allowed"])
        admin.press("重新允许播放")
        wait_for(lambda: snapshot()["devices"][member]["playback_allowed"])
        assert not ipc(senderdir, "status")["sender"]["running"]
        report["scenarios"]["native_device_disconnect_allow_requires_manual_restart"] = True
        admin.screenshot("device-authority")
        admin.close()
        app = Window(senderdir, "sender-ui-reopened")
        app.press("Sender")
        app.press("删除绑定…")
        wait_for(lambda: "取消" in app.ax("return entire contents of window 1 of p"))
        app.ax("key code 53")
        assert ipc(senderdir, "status")["output_binding"]["output_id"] == original_id
        report["scenarios"]["native_delete_confirmation_escape_keeps_binding"] = True
        report["passed"] = all(report["scenarios"].values())
    except Exception as error:
        report["failure"] = str(error)
        if windows:
            try:
                windows[-1].screenshot("failure")
                report["failure_ui_tree"] = windows[-1].ax("return entire contents of window 1 of p")
            except (OSError, RuntimeError):
                pass
        raise
    finally:
        for directory in (senderdir, hubdir):
            try:
                ipc(directory, "shutdown")
            except (OSError, RuntimeError):
                pass
        for window in windows:
            window.close()
        for child in children:
            try:
                child.wait(timeout=8)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        for handle in handles:
            handle.close()
        remove_owned_fixture(base)
        (out / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report, ensure_ascii=False))


if __name__ == "__main__":
    main()
