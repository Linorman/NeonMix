#!/usr/bin/env python3
"""Run the E00/E01 checks and save exact logs and exit codes inside the project."""
import json
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts' / 'checks'
OUT.mkdir(parents=True, exist_ok=True)
commands = {
    'format': ['cargo', 'fmt', '--all', '--', '--check'],
    'clippy': ['cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings'],
    'tests': ['cargo', 'test', '--workspace', '--locked'],
    'release': ['cargo', 'build', '--workspace', '--release', '--locked'],
    'simulate': [str(ROOT / 'target/release' / ('neonmix-audio.exe' if sys.platform == 'win32' else 'neonmix-audio')), 'simulate'],
}
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
