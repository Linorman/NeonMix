#!/usr/bin/env python3
"""Preserve checks and logs, with explicit full native-media coverage."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts' / 'checks'
OUT.mkdir(parents=True, exist_ok=True)
scope = ['--workspace']
parser = argparse.ArgumentParser()
parser.add_argument('--native-media', action='store_true', help='Check the complete workspace; requires the native GStreamer SDK')
parser.add_argument('--keep-going', action='store_true', help='Retain results of independent checks after a failure')
parser.add_argument('--only', nargs='+', help='Run only these named checks (for focused native CI retries)')
args = parser.parse_args()
if sys.platform != 'darwin' and not args.native_media:
    scope += ['--exclude', 'neonmix-media', '--exclude', 'neonmix-hub']
commands = {
    'format': ['cargo', 'fmt', '--all', '--', '--check'],
    'credential-dependencies': [sys.executable, str(ROOT / 'tools/credential_dependency_check.py')],
    'clippy': ['cargo', 'clippy', *scope, '--all-targets', '--locked', '--', '-D', 'warnings'],
    'tests': ['cargo', 'test', *scope, '--locked', *(['--no-fail-fast'] if args.keep_going else [])],
    'release': ['cargo', 'build', *scope, '--release', '--locked'],
    **({'credential-store': [sys.executable, str(ROOT / 'tools/credential_store_probe.py'), '--release']} if sys.platform == 'darwin' or args.native_media else {}),
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
if args.only:
    unknown = set(args.only) - commands.keys()
    if unknown:
        parser.error(f'Unknown checks: {sorted(unknown)}')
    commands = {name: commands[name] for name in args.only}
results = {}
for name, command in commands.items():
    started = time.monotonic()
    with (OUT / f'{name}.log').open('w') as log:
        try:
            completed = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            code = completed.returncode
            error = None
        except OSError as exception:
            code = None
            error = str(exception)
            log.write(error + '\n')
    results[name] = {'command': command, 'exit_code': code, 'seconds': round(time.monotonic() - started, 3)}
    if error:
        results[name]['error'] = error
    (OUT / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
    if name == 'release' and os.environ.get('GITHUB_OUTPUT'):
        with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as output:
            output.write(f'release={"success" if code == 0 else "failure"}\n')
    print(name, code, flush=True)
    if code != 0:
        print((OUT / f'{name}.log').read_text()[-6000:])
        if not args.keep_going:
            break
(OUT / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
raise SystemExit(0 if len(results) == len(commands) and all(r['exit_code'] == 0 for r in results.values()) else 1)
