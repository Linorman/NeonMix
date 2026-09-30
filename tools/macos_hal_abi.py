#!/usr/bin/env python3
"""Compare Rust AudioServerPlugIn layouts/constants against the installed Apple C SDK."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import shutil

ROOT = Path(__file__).resolve().parents[1]
if sys.platform != 'darwin':
    raise SystemExit('macOS SDK ABI check requires macOS')
out = ROOT / 'artifacts/e06-hal-abi'
out.mkdir(parents=True, exist_ok=True)
temp = Path(tempfile.mkdtemp(prefix='hal-abi-', dir=ROOT / '.local/tmp'))
fields = {
    'SMPTETime': ['mCounter'],
    'AudioTimeStamp': [],
    'AudioStreamBasicDescription': [],
    'AudioServerPlugInClientInfo': ['mBundleID'],
    'AudioServerPlugInIOCycleInfo': ['mInputTime', 'mOutputTime'],
    'AudioServerPlugInDriverInterface': ['StartIO', 'DoIOOperation'],
}
operations = ['Thread', 'Cycle', 'ReadInput', 'ConvertInput', 'ProcessInput', 'ProcessOutput', 'MixOutput', 'ProcessMix', 'ConvertMix', 'WriteMix']
statements = []
for kind, names in fields.items():
    statements.append(f'printf("{kind}.size %zu\\n",sizeof({kind}));')
    statements.extend(f'printf("{kind}.{name} %zu\\n",offsetof({kind},{name}));' for name in names)
statements.extend(f'printf("{name} %u\\n",(unsigned)kAudioServerPlugInIOOperation{name});' for name in operations)
source = '#include <CoreAudio/AudioServerPlugIn.h>\n#include <stdio.h>\n#include <stddef.h>\nint main(void){\n' + '\n'.join(statements) + '\n}\n'
try:
    (temp / 'sdk.c').write_text(source)
    subprocess.run(['xcrun', 'clang', str(temp / 'sdk.c'), '-o', str(temp / 'sdk')], check=True)
    sdk = {key: int(value) for key, value in (line.split() for line in subprocess.check_output([str(temp / 'sdk')], text=True).splitlines())}
    rust = json.loads(subprocess.check_output(['cargo', 'run', '--locked', '--quiet', '-p', 'neonmix-hal', '--example', 'abi_layout'], cwd=ROOT, text=True))
    differences = {key: {'sdk': value, 'rust': rust.get(key)} for key, value in sdk.items() if rust.get(key) != value}
    result = {'passed': not differences, 'sdk_version': subprocess.check_output(['xcrun', '--show-sdk-version'], text=True).strip(), 'sdk': sdk, 'rust': rust, 'differences': differences}
    (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
    if differences:
        raise SystemExit(1)
finally:
    shutil.rmtree(temp)
