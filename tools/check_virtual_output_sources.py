#!/usr/bin/env python3
"""Type-check the actual E06 platform selectors without the E02 native media SDK.

Runs only compilers on macOS. It never executes a Windows/Linux binary. Existing
target libraries and the project-local Linux sysroot are required; no installation.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
if sys.platform != 'darwin':
    raise SystemExit('E06 source cross-check is a macOS-only development probe')
OUT = ROOT / 'artifacts/e06-output-source-check'
OUT.mkdir(parents=True, exist_ok=True)
temporary = Path(tempfile.mkdtemp(prefix='e06-source-', dir=ROOT / '.local/tmp'))
try:
    (temporary / 'src').mkdir()
    manifest = '[workspace]\n[package]\nname="neonmix-e06-source-check"\nversion="0.1.0"\nedition="2024"\n[dependencies]\nserde={version="1.0",features=["derive"]}\n'
    for name, path in [('neonmix-core', 'crates/audio-core'), ('neonmix-output-binding', 'crates/output-binding'), ('neonmix-linux', 'adapters/linux'), ('neonmix-windows', 'adapters/windows')]:
        manifest += f'{name}={{path={json.dumps(str(ROOT / path))}}}\n'
    manifest += '[patch.crates-io]\ncpal={path=' + json.dumps(str(ROOT / 'vendor/cpal')) + '}\n'
    (temporary / 'Cargo.toml').write_text(manifest)
    shutil.copyfile(ROOT / 'Cargo.lock', temporary / 'Cargo.lock')
    binding_source = (ROOT / 'apps/hub/src/binding.rs').read_text()
    start = binding_source.index('pub fn sync_native_name(')
    end = binding_source.index('\npub struct Guard', start)
    (temporary / 'src/main.rs').write_text(
        'type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;\n'
        '#[path=' + json.dumps(str(ROOT / 'apps/hub/src/virtual_output.rs')) + '] mod virtual_output;\n'
        'mod binding {use neonmix_output_binding::OutputBinding; use crate::Result;\n' + binding_source[start:end] + '\n}\n'
        'fn main(){let _ = virtual_output::resolve(virtual_output::Provider::Neonmix, vec![]);'
        'let _sync: fn(&neonmix_output_binding::OutputBinding) -> Result<()> = binding::sync_native_name;}\n')
    results = {}
    for target in ['x86_64-pc-windows-gnu', 'x86_64-unknown-linux-gnu']:
        env = os.environ.copy()
        if 'linux' in target:
            sysroot = ROOT / '.local/cross-linux/sysroot'
            rust = ROOT / '.local/cross-linux/rust'
            if not sysroot.is_dir() or not rust.is_dir():
                raise RuntimeError('prepare the project-local Linux sysroot first')
            env['CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS'] = f'--sysroot={rust}'
            env['PKG_CONFIG_ALLOW_CROSS'] = '1'
            env['PKG_CONFIG_SYSROOT_DIR'] = str(sysroot)
            env['PKG_CONFIG_PATH'] = str(sysroot / 'usr/lib/x86_64-linux-gnu/pkgconfig')
            env['PKG_CONFIG_LIBDIR'] = env['PKG_CONFIG_PATH']
            flags = f'--target={target} --sysroot={sysroot} -I{sysroot}/usr/include/x86_64-linux-gnu'
            env['BINDGEN_EXTRA_CLANG_ARGS'] = flags
            env['CC_x86_64_unknown_linux_gnu'] = 'clang'
            env['CFLAGS_x86_64_unknown_linux_gnu'] = flags
            env['LIBCLANG_PATH'] = '/Library/Developer/CommandLineTools/usr/lib'
        command = ['cargo', 'clippy', '--offline', '--manifest-path', str(temporary / 'Cargo.toml'), '--target', target, '--', '-D', 'warnings']
        result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
        (OUT / (target + '.log')).write_text(result.stdout + result.stderr)
        results[target] = {'exit_code': result.returncode, 'executed_target_binary': False}
        if result.returncode:
            print(result.stderr[-5000:])
    sources = ['apps/hub/src/virtual_output.rs', 'apps/hub/src/binding.rs', 'adapters/linux/src/lib.rs', 'adapters/linux/src/owned_sink.rs', 'adapters/windows/src/virtual_output.rs']
    report = {'passed': all(value['exit_code'] == 0 for value in results.values()),
              'scope': 'macOS cross type-check of actual E06 platform selection and native name sync; no platform runtime',
              'targets': results, 'source_sha256': {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in sources}}
    (OUT / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))
    if not report['passed']:
        raise SystemExit(1)
finally:
    shutil.rmtree(temporary)
