#!/usr/bin/env python3
"""Preserve checks and logs; E02 native runtime is currently macOS only."""
import json
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts' / 'checks'
OUT.mkdir(parents=True, exist_ok=True)
scope = ['--workspace']
if sys.platform != 'darwin':
    scope += ['--exclude', 'neonmix-media', '--exclude', 'neonmix-hub']
commands = {
    'format': ['cargo', 'fmt', '--all', '--', '--check'],
    'clippy': ['cargo', 'clippy', *scope, '--all-targets', '--locked', '--', '-D', 'warnings'],
    'tests': ['cargo', 'test', *scope, '--locked'],
    'release': ['cargo', 'build', *scope, '--release', '--locked'],
    'simulate': [str(ROOT / 'target/release' / ('neonmix-audio.exe' if sys.platform == 'win32' else 'neonmix-audio')), 'simulate'],
}
if sys.platform == 'darwin':
    hub = str(ROOT / 'target/release/neonmix-hub')
    commands.update({
        'hal-sdk-abi': [sys.executable, str(ROOT / 'tools/macos_hal_abi.py')],
        'hal-bundle': [str(ROOT / 'tools/build_macos_hal.sh')],
        'hal-bundle-host': [sys.executable, str(ROOT / 'tools/macos_hal_bundle_probe.py')],
        'hal-vendor-format': ['cargo', 'fmt', '--manifest-path', 'vendor/tympan-aspl/Cargo.toml', '--', '--check'],
        'hal-vendor-tests': ['cargo', 'test', '--manifest-path', 'vendor/tympan-aspl/Cargo.toml', '--locked', '--', '--test-threads=1'],
        'media-runtime': [hub, 'runtime'],
        'media-dual': [hub, 'probe', '--seconds', '5', '--streams', '2'],
        'media-loss-replay': [hub, 'probe', '--seconds', '5', '--streams', '1', '--drop-every', '17', '--replay'],
        'media-reject': [hub, 'probe', '--seconds', '2', '--streams', '1', '--wrong-fingerprint'],
        'drift': ['cargo', 'test', '--release', '--locked', '-p', 'neonmix-core', '--test', 'drift_stability', '--', '--ignored', '--nocapture'],
    })
results = {}
for name, command in commands.items():
    started = time.monotonic()
    with (OUT / f'{name}.log').open('w') as log:
        completed = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
    results[name] = {'command': command, 'exit_code': completed.returncode, 'seconds': round(time.monotonic() - started, 3)}
    print(name, completed.returncode, flush=True)
    if completed.returncode:
        print((OUT / f'{name}.log').read_text()[-6000:])
        break
(OUT / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
raise SystemExit(0 if len(results) == len(commands) and all(r['exit_code'] == 0 for r in results.values()) else 1)
