#!/usr/bin/env python3
"""Run native lifecycle checks using named controls and preserve prior reports."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

import e07_native_ui_probe as probe
from macos_ui_visual_probe import VisualWindow, click_native_button, scroll_window

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary-dir", type=Path, default=ROOT / "target/release")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    probe.UI = args.binary_dir.resolve() / "neonmix-desktop"
    evidence = ROOT / "docs/evidence/e07/native-ui-macos.json"
    prior = evidence.read_bytes() if evidence.exists() else None

    class CompatibleNativeUI(probe.NativeUI):
        def __init__(self, directory):
            self.directory = directory
            self.log = (output / "native-runtime.log").open("a")
            binary = probe.UI
            self.process = subprocess.Popen([str(probe.DEV), str(binary), "--state-dir", str(directory)], stdout=self.log, stderr=self.log, start_new_session=True, cwd=ROOT)

        def ax(self, body):
            for label in ("Mixer", "诊断"):
                body = body.replace(f'checkbox "{label}"', f'button "{label}"')
            result = super().ax(body)
            if 'click checkbox "MacBook Pro Speakers"' in body:
                time.sleep(.3)
                super().ax("key code 53")
                time.sleep(.3)
            return result

        def activate(self):
            super().activate()
            VisualWindow.capture(self, output, "live-hub")

        def press(self, label):
            if label in ("总静音", "取消总静音", "导出脱敏诊断"):
                scroll_window(self, -30)
            click_native_button(self, label, last=label == "退出后台")
            if label in ("开始共享", "总静音", "取消总静音", "导出脱敏诊断"):
                VisualWindow.capture(self, output, "live-" + label)

    probe.NativeUI = CompatibleNativeUI
    try:
        probe.main()
    finally:
        if evidence.exists():
            result = json.loads(evidence.read_text())
            result["binary_sha256"] = hashlib.sha256(probe.UI.read_bytes()).hexdigest()
            result["probe_adaptations"] = ["Fields are located by their accessible names", "Named controls use native keyboard activation; the confirmation button is selected separately from the footer"]
            (output / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        if prior is None:
            evidence.unlink(missing_ok=True)
        else:
            evidence.write_bytes(prior)


if __name__ == "__main__":
    main()
