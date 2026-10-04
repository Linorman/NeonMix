#!/usr/bin/env python3
"""Windows native runtime evidence; absent audio hardware is an explicit skip."""
import ctypes
from ctypes import wintypes
import json
from pathlib import Path
import platform
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts/windows/runtime'
BIN = ROOT / 'target/release'


def desktop_probe():
    user32 = ctypes.WinDLL('user32', use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    pages = {}
    for page in ['hub', 'sender', 'mixer', 'devices', 'diagnostics']:
        with (OUT / f'desktop-{page}.log').open('w', encoding='utf-8') as log:
            process = subprocess.Popen([str(BIN / 'neonmix-desktop.exe'), '--preview-page', page],
                                       cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            windows = []
            @callback_type
            def collect(window, _):
                pid = wintypes.DWORD()
                user32.GetWindowThreadProcessId(window, ctypes.byref(pid))
                if pid.value == process.pid and user32.IsWindowVisible(window):
                    windows.append(window)
                return True
            try:
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline and process.poll() is None:
                    windows.clear()
                    user32.EnumWindows(collect, 0)
                    if windows:
                        break
                    time.sleep(0.2)
                if not windows:
                    raise RuntimeError(f'{page}: no visible native window, exit={process.poll()}')
                time.sleep(2)
                if process.poll() is not None:
                    raise RuntimeError(f'{page}: exited before the render smoke check')
                user32.PostMessageW(windows[0], 0x0010, 0, 0)  # WM_CLOSE, preview has no tray/background.
                code = process.wait(timeout=10)
                if code:
                    raise RuntimeError(f'{page}: window close failed, exit={code}')
                pages[page] = {'visible_window': True, 'survived_seconds': 2, 'graceful_close': True}
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
    return pages


def main():
    if sys.platform != 'win32':
        raise SystemExit('This probe requires native Windows execution.')
    OUT.mkdir(parents=True, exist_ok=True)
    report = {'platform': platform.platform(), 'machine': platform.machine(),
              'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'scenarios': {}, 'limitations': {
                  'physical_audio_soak': 'Not exercised without an explicitly selected physical endpoint.',
                  'driver_loading': 'Unsigned CI build does not establish installation or kernel runtime.',
                  'gui': 'Native window/render/startup/close smoke only; no interactive UX or screen-reader acceptance.'}}
    commands = {
        'file-credential-store': ['cargo', 'test', '--locked', '-p', 'neonmix-identity'],
        'credential-storage-cli': [sys.executable, str(ROOT / 'tools/credential_store_probe.py'), '--release'],
        'named-pipe': ['cargo', 'test', '--locked', '-p', 'neonmix-desktop-service', '--test', 'windows_native_ipc', '--', '--nocapture'],
        'wasapi-devices': [str(BIN / 'neonmix-audio.exe'), 'devices'],
        'media-runtime': [str(BIN / 'neonmix-hub.exe'), 'runtime'],
        'media-dual': [str(BIN / 'neonmix-hub.exe'), 'probe', '--seconds', '5', '--streams', '2'],
        'media-loss-replay': [str(BIN / 'neonmix-hub.exe'), 'probe', '--seconds', '5', '--streams', '1', '--drop-every', '17', '--replay'],
        'media-reject': [str(BIN / 'neonmix-hub.exe'), 'probe', '--seconds', '2', '--streams', '1', '--wrong-fingerprint'],
    }
    for rate in [44100, 48000, 96000]:
        commands[f'simulate-{rate}'] = [str(BIN / 'neonmix-audio.exe'), 'simulate', '--input-rate', str(rate), '--output-rate', '48000']
    for name, command in commands.items():
        started = time.monotonic()
        log_path = OUT / f'{name}.log'
        try:
            with log_path.open('w', encoding='utf-8') as log:
                completed = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=120)
            result = {'passed': completed.returncode == 0, 'exit_code': completed.returncode}
            if name == 'wasapi-devices' and not completed.returncode:
                devices = json.loads(log_path.read_text(encoding='utf-8'))
                result['devices'] = devices
                report['limitations']['physical_audio_soak'] = {'status': 'skipped', 'enumerated_devices': len(devices),
                    'reason': 'Hosted runner provides no explicitly selected physical audio fixture.'}
        except Exception as error:
            result = {'passed': False, 'error': str(error)}
        result.update(command=command, seconds=round(time.monotonic() - started, 3))
        report['scenarios'][name] = result
        print(name, result['passed'], flush=True)
    try:
        report['scenarios']['desktop-pages'] = {'passed': True, 'pages': desktop_probe()}
    except Exception as error:
        report['scenarios']['desktop-pages'] = {'passed': False, 'error': str(error)}
    report['passed'] = all(case['passed'] for case in report['scenarios'].values())
    (OUT / 'result.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
