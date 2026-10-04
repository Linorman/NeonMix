#!/usr/bin/env python3
"""Capture actual macOS egui windows at normal/minimum sizes.

Recorded-state previews do not start background audio; screenshots and AX trees
are evidence of rendering, while production lifecycle is tested separately.
"""
import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import subprocess
import time

from e07_background_probe import wait_for
from e07_native_ui_probe import NativeUI

ROOT = Path(__file__).resolve().parents[1]
DEV = ROOT / "tools/dev"
UI = ROOT / "target/release/neonmix-desktop"


def scroll_window(app, lines):
    """Use native wheel events inside the foreground test window."""
    class Point(ctypes.Structure):
        _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]
    cg = ctypes.CDLL("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")
    cf = ctypes.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
    cg.CGEventCreateMouseEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint32, Point, ctypes.c_uint32]
    cg.CGEventCreateMouseEvent.restype = ctypes.c_void_p
    cg.CGEventCreateScrollWheelEvent.restype = ctypes.c_void_p
    cg.CGEventCreateScrollWheelEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_int32]
    cg.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
    cf.CFRelease.argtypes = [ctypes.c_void_p]
    app.ax("set frontmost of p to true")
    bounds = app.ax('set xy to position of window 1 of p\nset wh to size of window 1 of p\nreturn (item 1 of xy as text) & "," & (item 2 of xy as text) & "," & (item 1 of wh as text) & "," & (item 2 of wh as text)')
    x, y, width, height = map(int, bounds.split(","))
    move = cg.CGEventCreateMouseEvent(None, 5, Point(x + width/2, y + height/2), 0)
    wheel = cg.CGEventCreateScrollWheelEvent(None, 1, 1, lines)
    for event in (move, wheel):
        cg.CGEventPost(0, event)
        cf.CFRelease(event)
    time.sleep(.2)


def click_native_button(app, label, last=False):
    """Activate a named control by keyboard before falling back to mouse input."""
    target = f'(last button of group 1 of window 1 of p whose name is "{label}")' if last else f'button "{label}" of group 1 of window 1 of p'
    try:
        wait_for(lambda: app.ax(f'return enabled of {target}') == "true")
        app.ax(f'''set frontmost of p to true
set value of attribute "AXFocused" of {target} to true
delay 0.2
key code 49''')
        time.sleep(.3)
        return
    except RuntimeError:
        pass
    class Point(ctypes.Structure):
        _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]
    cg = ctypes.CDLL("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")
    cf = ctypes.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
    cg.CGEventCreateMouseEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint32, Point, ctypes.c_uint32]
    cg.CGEventCreateMouseEvent.restype = ctypes.c_void_p
    cg.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
    cf.CFRelease.argtypes = [ctypes.c_void_p]
    target = f'(last button of group 1 of window 1 of p whose name is "{label}")' if last else f'button "{label}" of group 1 of window 1 of p'
    for _ in range(24):
        app.ax("set frontmost of p to true")
        try:
            values = app.ax(f'''set e to {target}
if not enabled of e then return "disabled"
set xy to position of e
set wh to size of e
set wp to position of window 1 of p
set ws to size of window 1 of p
return (item 1 of xy as text) & "," & (item 2 of xy as text) & "," & (item 1 of wh as text) & "," & (item 2 of wh as text) & "," & (item 2 of wp as text) & "," & (item 2 of ws as text)''')
            if values == "disabled":
                time.sleep(.3)
                continue
            x, y, width, height, top, window_height = map(float, values.split(","))
            if y + height > top + window_height - 75 and label not in ("退出后台", "隐藏窗口", "停止发送"):
                scroll_window(app, -4)
                continue
            if y < top + 90 and label not in ("Hub 设置", "Sender", "Mixer", "设备管理", "诊断"):
                scroll_window(app, 4)
                continue
            point = Point(x + width/2, y + height/2)
            for kind in (5, 1, 2):
                event = cg.CGEventCreateMouseEvent(None, kind, point, 0)
                cg.CGEventPost(0, event)
                cf.CFRelease(event)
                time.sleep(.06)
            time.sleep(.3)
            return
        except RuntimeError:
            time.sleep(.2)
    raise RuntimeError("No enabled visible button: " + label)


class VisualWindow(NativeUI):
    def __init__(self, output, page, width, height, fixture=None):
        self.log = (output / f"{page}-{width}.log").open("w")
        command = [str(DEV), str(UI), "--preview-page", page, "--width", str(width), "--height", str(height)]
        if fixture:
            command += ["--preview-data", str(fixture)]
        self.process = subprocess.Popen(command, cwd=ROOT, stdout=self.log, stderr=self.log)
        wait_for(lambda: self.ax("set frontmost of p to true\nreturn count of windows of p") == "1")
        self.ax("return entire contents of window 1 of p")
        time.sleep(.8)

    def capture(self, output, name):
        self.ax("set frontmost of p to true")
        time.sleep(.3)
        bounds = self.ax("set xy to position of window 1 of p\nset wh to size of window 1 of p\nreturn (item 1 of xy as text) & \",\" & (item 2 of xy as text) & \",\" & (item 1 of wh as text) & \",\" & (item 2 of wh as text)")
        completed = subprocess.run([str(DEV), "/usr/sbin/screencapture", "-x", "-R" + bounds, str(output / (name + ".png"))], cwd=ROOT, capture_output=True, text=True)
        if completed.returncode:
            raise RuntimeError(completed.stderr)
        (output / (name + "-ax.txt")).write_text(self.ax("return entire contents of window 1 of p"))
        return bounds


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--fixture", type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"scope": "actual native window rendering with recorded public state; preview actions do not commit to a server", "binary_sha256": hashlib.sha256(UI.read_bytes()).hexdigest(), "screens": {}, "checks": {}, "errors": {}}
    for width, height in ((1100, 760), (600, 440)):
        for page in ("hub", "sender", "mixer", "devices", "diagnostics"):
            app = None
            name = f"{page}-{width}"
            try:
                app = VisualWindow(output, page, width, height, args.fixture)
                report["screens"][name] = {"bounds": app.capture(output, name), "fixture": str(args.fixture) if args.fixture else None}
                if page == "hub" and width == 1100:
                    draft = "测试影音室 — 中文草稿 ABC 123"
                    app.paste("房间名称", draft)
                    app.press("Sender")
                    app.press("Hub 设置")
                    assert app.ax('return value of text field "房间名称" of group 1 of window 1 of p') == draft
                    report["checks"]["chinese_paste_and_draft_preserved_across_navigation"] = True
                    app.ax('set value of attribute "AXFocused" of button "Sender" of group 1 of window 1 of p to true\nkey code 49')
                    wait_for(lambda: "设备名称" in app.ax("return entire contents of window 1 of p"))
                    report["checks"]["navigation_button_keyboard_space_activation"] = True
                    app.capture(output, "keyboard-navigation")
                if page == "devices" and width == 1100 and args.fixture:
                    app.paste("搜索设备", "没有这个设备-验收测试")
                    wait_for(lambda: "无匹配结果" in app.ax("return entire contents of window 1 of p"))
                    app.capture(output, "devices-no-results")
                    app.press("清除")
                    assert app.ax('return value of text field "搜索设备" of group 1 of window 1 of p') == ""
                    report["checks"]["device_search_no_results_and_clear"] = True
            except Exception as error:
                report["errors"][name] = str(error)
            finally:
                if app:
                    app.close()
                (output / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    report["passed"] = len(report["screens"]) == 10 and not report["errors"]
    (output / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report, ensure_ascii=False))
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
