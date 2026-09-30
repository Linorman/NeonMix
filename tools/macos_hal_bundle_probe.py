#!/usr/bin/env python3
"""Verify and load the staged bundle without writing to the system HAL directory."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
if sys.platform != 'darwin':
    raise SystemExit('macOS bundle probe requires macOS')
BUNDLE = ROOT / 'artifacts/macos/NeonMixHAL.driver'
OUT = ROOT / 'artifacts/e06-hal-bundle'
OUT.mkdir(parents=True, exist_ok=True)
temporary = Path(tempfile.mkdtemp(prefix='hal-bundle-', dir=ROOT / '.local/tmp'))
try:
    subprocess.run(['codesign', '--verify', '--strict', str(BUNDLE)], check=True)
    identity = subprocess.run(['codesign', '-dvvv', '--entitlements', ':-', str(BUNDLE)], capture_output=True, text=True, check=True)
    (OUT / 'signature.log').write_text(identity.stdout + identity.stderr)
    host = temporary / 'hal-host'
    subprocess.run(['xcrun', 'clang', '-Wall', '-Wextra', '-Werror', str(ROOT / 'tools/macos_hal_host.c'), '-framework', 'CoreAudio', '-framework', 'CoreFoundation', '-o', str(host)], check=True)
    completed = subprocess.run([str(host), str(BUNDLE)], capture_output=True, text=True)
    (OUT / 'host.log').write_text(completed.stdout + completed.stderr)
    completed.check_returncode()
    result = json.loads(completed.stdout)
    result['binary_sha256'] = hashlib.sha256((BUNDLE / 'Contents/MacOS/NeonMixHAL').read_bytes()).hexdigest()
    result['sdk_version'] = subprocess.check_output(['xcrun', '--show-sdk-version'], text=True).strip()
    (OUT / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
finally:
    shutil.rmtree(temporary)
