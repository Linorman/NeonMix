#!/usr/bin/env python3
"""macOS Clang -> Windows x64 COFF compilation using project-local Microsoft WDK/SDK.
This is a compiler/header check, not MSVC linking, kernel loading, HLK or HVCI testing.
"""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import struct
import sys
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
if sys.platform != 'darwin':
    raise SystemExit('This header check is for the macOS development host')
driver = ROOT / 'drivers/windows/wavert'
out = ROOT / 'artifacts/windows-driver-header-check'
out.mkdir(parents=True, exist_ok=True)
packages = ROOT / '.local/windows-driver-sdk'
version = '10.0.26100.6584'
wdk = packages / f'Microsoft.Windows.WDK.x64.{version}' / 'c/Include'
sdk = packages / f'Microsoft.Windows.SDK.CPP.{version}' / 'c/Include/10.0.26100.0'
if not (sdk / 'shared/ntdef.h').exists():
    raise SystemExit('Restore the locked SDK with tools/prepare_windows_driver.py first')
compat = out / 'clang-kmdf-1.33'
shutil.copytree(wdk / 'wdf/kmdf/1.33', compat, dirs_exist_ok=True)
request = compat / 'wdfrequest.h'
original = request.read_bytes()
old = b'typedef enum _WDF_REQUEST_TYPE {'
new = b'typedef enum _WDF_REQUEST_TYPE : int {'
if original.count(old) != 1:
    raise RuntimeError('Unexpected WDF header; review compatibility adjustment')
# WDF's forward declaration explicitly fixes this enum to int, but its definition
# omits the type. MSVC accepts it; Clang rejects it. The overlay repeats that same
# int underlying type without changing values/layout; native MSBuild uses the original.
request.write_bytes(original.replace(old, new))
report = {'scope': 'macOS compiler/header check; not Windows driver acceptance',
          'compiler': subprocess.check_output(['xcrun', 'clang++', '--version'], text=True).splitlines()[0],
          'wdk_sdk_version': version,
          'compatibility_overlay': {'original_sha256': hashlib.sha256(original).hexdigest(),
                                    'adjusted_sha256': hashlib.sha256(request.read_bytes()).hexdigest(),
                                    'change': 'WDF_REQUEST_TYPE definition explicitly uses the already-declared int type'},
          'files': [], 'passed': False}
resource = Path(subprocess.check_output(['xcrun', 'clang', '-print-resource-dir'], text=True).strip())
includes = [resource / 'include', wdk / '10.0.26100.0/km', wdk / '10.0.26100.0/km/crt', compat,
            sdk / 'shared', sdk / 'um', sdk / 'ucrt'] + [driver / 'Source' / path for path in ('Inc', 'Main', 'Filters', 'Utilities')]
base = ['xcrun', 'clang++', '--target=x86_64-pc-windows-msvc', '-fms-extensions', '-fms-compatibility',
        '-fms-compatibility-version=19.40', '-std=c++17', '-fno-exceptions', '-fno-rtti', '-O2',
        '-Xclang', '-cfguard',
        '-Wno-ignored-attributes', '-Wno-ignored-pragma-intrinsic', '-Wno-pragma-pack', '-Wno-microsoft-enum-value',
        '-Wno-nonportable-include-path', '-Wno-implicit-exception-spec-mismatch', '-Wno-writable-strings',
        '-D_AMD64_', '-D_WIN64', '-D_KERNEL_MODE', '-D_USE_WAVERT_', '-D_USE_IPortClsRuntimePower',
        '-D_NEW_DELETE_OPERATORS_', '-DPC_IMPLEMENTATION', '-DUNICODE', '-D_UNICODE']
for path in includes:
    base += ['-I', str(path)]
try:
    tree = ET.parse(driver / 'NeonMixAudio.vcxproj')
    files = [driver / item.attrib['Include'].replace('\\', '/')
             for item in tree.findall('.//{*}ClCompile') if 'Include' in item.attrib]
    for source in files:
        obj = out / (source.parent.name + '-' + source.stem + '.obj')
        command = base + ['-c', str(source), '-o', str(obj)]
        result = subprocess.run(command, capture_output=True, text=True)
        (out / (obj.stem + '.log')).write_text(result.stdout + result.stderr)
        report['files'].append({'source': str(source.relative_to(ROOT)), 'exit_code': result.returncode,
                                'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                                'object_sha256': hashlib.sha256(obj.read_bytes()).hexdigest() if result.returncode == 0 else None})
        print(source.name, result.returncode, flush=True)
        if result.returncode:
            print(result.stderr[-5000:])
    compiled = bool(files) and all(item['exit_code'] == 0 for item in report['files'])
    if compiled:
        # Reuse the installed LLVM linker read-only; import libraries are the locked WDK.
        rust = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
        linker = rust / 'lib/rustlib/aarch64-apple-darwin/bin/rust-lld'
        lib = packages / f'Microsoft.Windows.WDK.x64.{version}' / 'c/Lib'
        km = lib / '10.0.26100.0/km/x64'
        wdf = lib / 'wdf/kmdf/x64/1.33'
        image = out / 'NeonMixAudio.sys'
        objects = [out / (source.parent.name + '-' + source.stem + '.obj') for source in files]
        args = [str(linker), '-flavor', 'link', '/driver', '/subsystem:native', '/entry:FxDriverEntry',
                '/nodefaultlib', '/machine:x64', '/dynamicbase', '/nxcompat', '/guard:cf', f'/out:{image}']
        args += [str(obj) for obj in objects]
        args += [str(km / name) for name in ('ntoskrnl.lib', 'hal.lib', 'portcls.lib', 'ksguid.lib', 'stdunk.lib',
                                            'wmilib.lib', 'bufferoverflowfastfailk.lib', 'libcntpr.lib')]
        args += [str(wdf / name) for name in ('wdfdriverentry.lib', 'wdfldr.lib')]
        linked = subprocess.run(args, capture_output=True, text=True)
        (out / 'link.log').write_text(linked.stdout + linked.stderr)
        report['link_exit_code'] = linked.returncode
        if linked.returncode:
            print(linked.stderr[-4000:])
        else:
            data = image.read_bytes()
            offset = struct.unpack_from('<I', data, 0x3c)[0]
            machine = struct.unpack_from('<H', data, offset + 4)[0]
            magic = struct.unpack_from('<H', data, offset + 24)[0]
            subsystem, flags = struct.unpack_from('<HH', data, offset + 24 + 68)
            report['pe'] = {'machine': machine, 'optional_header_magic': magic, 'subsystem': subsystem,
                            'dll_characteristics': flags, 'sha256': hashlib.sha256(data).hexdigest(),
                            'signed': False, 'msvc_built': False, 'windows_loaded': False}
            report['passed'] = bool(data[offset:offset+4] == b'PE\0\0' and machine == 0x8664 and magic == 0x20b
                                     and subsystem == 1 and flags & 0x100 and flags & 0x40 and flags & 0x4000)
finally:
    shutil.rmtree(compat)
    (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
raise SystemExit(0 if report['passed'] else 1)
