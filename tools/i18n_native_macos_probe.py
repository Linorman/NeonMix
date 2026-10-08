#!/usr/bin/env python3
"""Native macOS i18n/tray/keyboard probe of the screenshot-only preview.

Never connects to background audio. All state, logs and captures stay in the
project. This does not certify VoiceOver speech or native IME composition.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time
import uuid

from e07_background_probe import wait_for
from e07_native_ui_probe import NativeUI

ROOT = Path(__file__).resolve().parents[1]
DEV = ROOT / "tools/dev"
UI = ROOT / "artifacts/i18n/preview/neonmix-desktop"
COMBO = 'pop up button "语言 / Language" of group 1 of window 1 of p'
TRAY = 'menu bar item 1 of menu bar 2 of p'
LABELS = {
    "zh-CN": {"about": "关于", "show": "打开 NeonMix", "stop": "停止发送", "quit": "退出后台…", "hide": "关闭窗口", "auto": "Auto（跟随系统）", "cancel": "取消"},
    "en": {"about": "About", "show": "Open NeonMix", "stop": "Stop sending", "quit": "Quit background…", "hide": "Close window", "auto": "Auto (System)", "cancel": "Cancel"},
}


def quote(value):
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


class PreviewWindow(NativeUI):
    def __init__(self, output, language="zh-CN", candidates=None, tray=True):
        self.directory = output / ("state-" + uuid.uuid4().hex[:8])
        self.directory.mkdir(mode=0o700)
        self.log = (output / (self.directory.name + ".log")).open("w")
        env = os.environ.copy()
        for key in ("NEONMIX_SCREENSHOT_TO", "NEONMIX_SCREENSHOT_UNDO", "NEONMIX_SCREENSHOT_CONFIRM", "NEONMIX_SCREENSHOT_PALETTE", "NEONMIX_SCREENSHOT_BUSY"):
            env.pop(key, None)
        if tray:
            env["NEONMIX_SCREENSHOT_TRAY"] = "1"
        else:
            env.pop("NEONMIX_SCREENSHOT_TRAY", None)
        command = [str(DEV), str(UI), "--preview-page", "about", "--preview-language", language,
                   "--state-dir", str(self.directory), "--width", "1100", "--height", "760"]
        for candidate in candidates or ["zh-Hans-CN", "en-US"]:
            command += ["--preview-system-locale", candidate]
        self.process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=self.log, stderr=self.log, start_new_session=True)
        try:
            wait_for(lambda: self.ax("set frontmost of p to true\nreturn count of windows of p") == "1", seconds=15)
            wait_for(lambda: self.ax(f"return exists {COMBO}") == "true", seconds=15)
        except Exception:
            self.close()
            shutil.rmtree(self.directory, ignore_errors=True)
            raise

    def tree(self):
        return self.ax("return entire contents of window 1 of p")

    def check_locale(self, locale):
        target = f'checkbox {quote(LABELS[locale]["about"])} of group 1 of window 1 of p'
        wait_for(lambda: self.ax(f"return exists {target}") == "true")
        return self.tree()

    def open_language(self):
        self.ax(f'set frontmost of p to true\nset value of attribute "AXFocused" of {COMBO} to true\ndelay 0.1\nkey code 49')
        try:
            wait_for(lambda: self.ax('return exists checkbox "English" of group 1 of window 1 of p') == "true", seconds=2)
            return "keyboard-space"
        except RuntimeError:
            self.ax(f"click {COMBO}")
            wait_for(lambda: self.ax('return exists checkbox "English" of group 1 of window 1 of p') == "true")
            return "AXPress"

    def select(self, preference, current_locale):
        method = self.open_language()
        choice = {"zh-CN": "简体中文", "en": "English", "auto": LABELS[current_locale]["auto"]}[preference]
        target = f'checkbox {quote(choice)} of group 1 of window 1 of p'
        self.ax(f'set value of attribute "AXFocused" of {target} to true\ndelay 0.1\nkey code 49')
        try:
            wait_for(lambda: self.ax('return exists checkbox "English" of group 1 of window 1 of p') == "false")
        except RuntimeError as error:
            raise RuntimeError(f"Language popup remained open after keyboard Space selected {preference}") from error
        effective = "zh-CN" if preference == "auto" else preference
        self.check_locale(effective)
        wait_for(lambda: self.ax(f'return value of attribute "AXFocused" of {COMBO}') == "true")
        return effective, method

    def menu(self, locale, capture=None):
        self.ax(f"click {TRAY}")
        wait_for(lambda: LABELS[locale]["show"] in self.ax("return entire contents of menu bar 2 of p"))
        names = self.ax(f"return name of every menu item of menu 1 of {TRAY}")
        assert all(LABELS[locale][kind] in names for kind in ("show", "stop", "quit")), names
        count = int(self.ax("return count of menu bar items of menu bar 2 of p"))
        assert count == 1, f"duplicate tray items: {count}"
        if capture:
            self.capture(capture, f"menu 1 of {TRAY}")
        self.ax("key code 53")
        return {"names": names, "tray_count": count}

    def application_menu(self, locale, capture=None):
        # Cocoa names the app menu after the raw executable during this preview;
        # the installed .app has CFBundleName NeonMix. Locate by its authored
        # actions rather than assuming the preview's platform-owned menu title.
        self.ax("set frontmost of p to true")
        time.sleep(.15)
        names = self.ax("return name of every menu bar item of menu bar 1 of p")
        count = int(self.ax("return count of menu bar items of menu bar 1 of p"))
        indices = [self.application_menu_index] if hasattr(self, "application_menu_index") else range(1, count + 1)
        for index in indices:
            target = f"menu bar item {index} of menu bar 1 of p"
            self.ax(f"click {target}")
            try:
                items = self.ax(f"return name of every menu item of menu 1 of {target}")
                if all(LABELS[locale][kind] in items for kind in ("show", "hide", "quit")):
                    self.application_menu_index = index
                    if capture:
                        self.capture(capture, f"menu 1 of {target}")
                    return {"native_bar_names": names, "target_index": index, "items": items}
            finally:
                self.ax("key code 53")
        raise RuntimeError("No application menu with localized Show/Hide/Quit actions: " + names)

    def capture(self, path, target="window 1 of p"):
        def rectangle(element):
            text = self.ax(f'set xy to position of {element}\nset wh to size of {element}\nreturn (item 1 of xy as text) & "," & (item 2 of xy as text) & "," & (item 1 of wh as text) & "," & (item 2 of wh as text)')
            values = [round(float(value)) for value in text.split(",")]
            return values if len(values) == 4 and values[2] > 0 and values[3] > 0 else None
        def bounds_ready():
            try:
                value = rectangle(target)
                if value:
                    return value
            except RuntimeError:
                if not target.startswith("menu "):
                    raise
            if target.startswith("menu "):
                first = rectangle(f"menu item 1 of {target}")
                last = rectangle(f"last menu item of {target}")
                if first and last:
                    return [first[0], first[1], max(first[2], last[2]), last[1] + last[3] - first[1]]
            return None
        # Native menu AX names can precede their animated nonzero geometry.
        bounds = ",".join(map(str, wait_for(bounds_ready, seconds=5)))
        result = subprocess.run([str(DEV), "/usr/sbin/screencapture", "-x", "-R" + bounds, str(path)], cwd=ROOT, capture_output=True, text=True, timeout=10)
        if result.returncode:
            raise RuntimeError(result.stderr.strip() + " (rect=" + bounds + ")")
        return bounds

    def sample(self):
        children = subprocess.run([str(DEV), "/usr/bin/pgrep", "-P", str(self.process.pid)], cwd=ROOT, capture_output=True, text=True, timeout=5)
        assert children.returncode in (0, 1), children.stderr.strip()
        assert not children.stdout.strip(), f"preview launched children: {children.stdout.strip()}"
        files = [str(path.relative_to(self.directory)) for path in self.directory.rglob("*")]
        assert not files, f"preview wrote persistent files: {files}"
        rss = subprocess.check_output([str(DEV), "/bin/ps", "-o", "rss=", "-p", str(self.process.pid)], cwd=ROOT, text=True, timeout=5).strip()
        return {"rss_kib": int(rss), "children": [], "state_files": []}


def main():
    global UI
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "artifacts/i18n/native-macos")
    parser.add_argument("--cycles", type=int, default=54)
    parser.add_argument("--binary", type=Path, default=UI, help="Frozen screenshot-feature preview binary inside the project")
    args = parser.parse_args()
    assert platform.system() == "Darwin", "macOS-only probe"
    assert args.cycles >= 50, "at least 50 language changes are required"
    output = args.output.resolve()
    output.relative_to(ROOT)
    output.mkdir(parents=True, exist_ok=True)
    UI = args.binary.resolve()
    UI.relative_to(ROOT)
    binary = UI.read_bytes()
    markers = [b"NEONMIX_SCREENSHOT_TO", b"NEONMIX_SCREENSHOT_TRAY"]
    missing = [marker.decode() for marker in markers if marker not in binary]
    if missing:
        failure = {"passed": False, "failure": "Binary lacks screenshot preview feature markers: " + ", ".join(missing), "binary": str(UI.relative_to(ROOT))}
        (output / "result.json").write_text(json.dumps(failure, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps(failure, ensure_ascii=False))
        raise SystemExit(1)
    report = {"platform": "macOS", "scope": "native GUI, keyboard, AX names and actual native menus on an isolated screenshot preview; no background audio",
              "binary": str(UI.relative_to(ROOT)), "binary_sha256": hashlib.sha256(binary).hexdigest(), "auto_candidates": ["zh-Hans-CN", "en-US"], "passed": False,
              "checks": {}, "cycles": [], "limits": ["VoiceOver speech was not listened to", "Real IME composition was not tested", "Preview is not a playing audio session", "Windows and Linux native behavior was not tested", "Auto uses injected preview candidates; actual system candidate collection is a separate check", "AX menu counts prove one visible tray; Rust menu handle identity is additionally guaranteed by the implementation"]}
    app = None
    try:
        app = PreviewWindow(output)
        current = "zh-CN"
        app.check_locale(current)
        (output / "zh-CN-window-ax.txt").write_text(app.tree())
        app.capture(output / "zh-CN-window.png")
        report["checks"]["zh-CN-tray"] = app.menu(current, output / "zh-CN-tray.png")
        report["checks"]["zh-CN-app-menu"] = app.application_menu(current, output / "zh-CN-app-menu.png")
        current, method = app.select("en", current)
        (output / "en-window-ax.txt").write_text(app.tree())
        app.capture(output / "en-window.png")
        report["checks"]["en-tray"] = app.menu(current, output / "en-tray.png")
        report["checks"]["en-app-menu"] = app.application_menu(current, output / "en-app-menu.png")
        current, _ = app.select("zh-CN", current)
        current, _ = app.select("auto", current)
        report["checks"]["all_three_preferences_native_keyboard"] = True
        start = app.sample()
        for index in range(args.cycles):
            preference = "en" if index % 2 == 0 else "auto"
            current, method = app.select(preference, current)
            menu = app.menu(current)
            sample = app.sample()
            report["cycles"].append({"index": index + 1, "preference": preference, "effective": current, "open_method": method, "menu": menu, **sample})
            if (index + 1) % 10 == 0:
                print(f"native i18n switches {index + 1}/{args.cycles}", flush=True)
                (output / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        end = app.sample()
        report["checks"]["rss_after_font_settle"] = {"start_kib": start["rss_kib"], "end_kib": end["rss_kib"], "delta_kib": end["rss_kib"] - start["rss_kib"], "observations": "Inspect cycle samples; a bounded RSS change is not an allocation leak proof."}
        report["checks"]["no_preview_audio_owner_or_preference_writes"] = True
        app.open_language()
        (output / "language-popup-ax.txt").write_text(app.tree())
        app.capture(output / "language-popup.png")
        app.ax("key code 53")
        wait_for(lambda: app.ax('return exists checkbox "English" of group 1 of window 1 of p') == "false")
        wait_for(lambda: app.ax(f'return value of attribute "AXFocused" of {COMBO}') == "true")
        report["checks"]["language_popup_escape_restores_selector_focus"] = True
        app.ax('set frontmost of p to true\nkeystroke "w" using command down')
        wait_for(lambda: app.ax("return count of windows of p") == "0")
        app.ax(f"click {TRAY}")
        wait_for(lambda: LABELS[current]["show"] in app.ax("return entire contents of menu bar 2 of p"))
        app.ax(f'click menu item {quote(LABELS[current]["show"])} of menu 1 of {TRAY}')
        wait_for(lambda: app.ax("return count of windows of p") == "1")
        app.check_locale(current)
        report["checks"]["cmd_w_hides_and_single_tray_restores"] = True
        app.ax('set frontmost of p to true\nkeystroke "q" using command down')
        wait_for(lambda: app.ax(f'return count of (buttons of group 1 of window 1 of p whose name is {quote(LABELS[current]["cancel"])})') == "1")
        (output / "quit-confirmation-ax.txt").write_text(app.tree())
        app.capture(output / "quit-confirmation.png")
        app.ax("key code 53")
        wait_for(lambda: app.ax(f'return exists button {quote(LABELS[current]["cancel"])} of group 1 of window 1 of p') == "false")
        assert app.process.poll() is None
        report["checks"]["cmd_q_opens_one_confirmation_and_escape_cancels"] = True
        app.sample()
        app.close()
        shutil.rmtree(app.directory, ignore_errors=True)
        app = None
        startup_cases = [(["en-US"], "en"), (["zh-TW", "en-GB"], "en"), (["fr-FR", "zh-Hans-CN", "en-US"], "zh-CN")]
        report["checks"]["auto_native_startup"] = []
        for index, (candidates, expected) in enumerate(startup_cases):
            app = PreviewWindow(output, language="auto", candidates=candidates, tray=False)
            app.check_locale(expected)
            (output / f"auto-startup-{index}-ax.txt").write_text(app.tree())
            app.capture(output / f"auto-startup-{index}.png")
            app.sample()
            report["checks"]["auto_native_startup"].append({"candidates": candidates, "effective": expected})
            app.close()
            shutil.rmtree(app.directory, ignore_errors=True)
            app = None
        assert hashlib.sha256(UI.read_bytes()).hexdigest() == report["binary_sha256"], "Frozen binary changed during native verification"
        report["passed"] = True
    except Exception as error:
        report["failure"] = str(error)
        if app:
            try:
                (output / "failure-ax.txt").write_text(app.tree())
            except (RuntimeError, subprocess.TimeoutExpired):
                pass
    finally:
        if app:
            app.close()
            shutil.rmtree(app.directory, ignore_errors=True)
        (output / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "completed_switches": len(report["cycles"]), "failure": report.get("failure")}, ensure_ascii=False))
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
