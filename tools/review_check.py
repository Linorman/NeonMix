#!/usr/bin/env python3
"""Light deterministic stability gate. Run through tools/dev (or dev.ps1).

This gate does not claim hardware, native media, installer or soak acceptance.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
PACKAGES = ['neonmix-core', 'neonmix-control', 'neonmix-airplay-adapter',
            'neonmix-airplay-ipc', 'neonmix-output-binding', 'neonmix-i18n',
            'neonmix-desktop-service', 'neonmix-lifecycle']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--with-apps', action='store_true',
                        help='also check Hub/Desktop; requires platform native SDKs')
    parser.add_argument('--output-dir', type=Path,
                        default=ROOT / 'artifacts/review-checks',
                        help='project-local directory for this run (isolate concurrent runs)')
    args = parser.parse_args()
    packages = PACKAGES + (['neonmix-hub', 'neonmix-desktop'] if args.with_apps else [])
    scope = [arg for package in packages for arg in ['-p', package]]
    commands = {
        'format': ['cargo', 'fmt', '--all', '--', '--check'],
        'packaging': [sys.executable, '-m', 'unittest', 'discover',
                      '-s', 'tools/tests', '-p', 'test_*.py'],
        'clippy': ['cargo', 'clippy', *scope, '--all-targets', '--locked', '--', '-D', 'warnings'],
        'tests': ['cargo', 'test', *scope, '--locked'],
    }
    directory = (ROOT / args.output_dir).resolve()
    if not directory.is_relative_to(ROOT):
        parser.error('--output-dir must be inside the project')
    directory.mkdir(parents=True, exist_ok=True)
    results = {}
    for name, command in commands.items():
        start = time.monotonic()
        with (directory / f'{name}.log').open('w') as log:
            result = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        results[name] = {'command': command, 'exit_code': result.returncode,
                         'seconds': round(time.monotonic() - start, 3)}
        (directory / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
        print(f'{name}: {result.returncode}', flush=True)
        if result.returncode:
            print((directory / f'{name}.log').read_text()[-5000:])
            return result.returncode
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
