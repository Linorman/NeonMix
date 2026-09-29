#!/usr/bin/env python3
"""Read-only environment report; GStreamer absence is explicit, not a false E02 pass."""
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
report = {'platform': platform.platform(), 'architecture': platform.machine(), 'native_baseline': tomllib.loads((ROOT / 'native-dependencies.toml').read_text()), 'tools': {}}
for name, args in {'rustc': ['-Vv'], 'cargo': ['-V'], 'pkg-config': ['--modversion', 'libpipewire-0.3'], 'gst-inspect-1.0': ['--version'], 'xcrun': ['--show-sdk-version']}.items():
    executable = shutil.which(name)
    if executable:
        result = subprocess.run([executable, *args], capture_output=True, text=True)
        report['tools'][name] = {'exit_code': result.returncode, 'stdout': result.stdout.strip(), 'stderr': result.stderr.strip()}
    else:
        report['tools'][name] = {'available': False}
print(json.dumps(report, indent=2))
