#!/usr/bin/env python3
"""Preserve native binaries, debug symbols, dependency licenses and source hashes."""
import hashlib
import json
import os
import platform
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
NAME = f'neonmix-e01-{sys.platform}-{platform.machine()}'
OUT = ROOT / 'artifacts' / NAME
OUT.mkdir(parents=True, exist_ok=True)


def command(args):
    try:
        result = subprocess.run(args, cwd=ROOT, capture_output=True, text=True)
    except OSError as error:
        return {'command': args, 'exit_code': None, 'stdout': '', 'stderr': str(error)}
    return {'command': args, 'exit_code': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}


metadata = command(['cargo', 'metadata', '--locked', '--format-version', '1'])
if metadata['exit_code']:
    raise RuntimeError(metadata['stderr'])
metadata = json.loads(metadata['stdout'])
(OUT / 'cargo-metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
licenses = [{'name': p['name'], 'version': p['version'], 'license': p['license'], 'source': p['source']} for p in metadata['packages']]
(OUT / 'dependency-licenses.json').write_text(json.dumps(licenses, indent=2) + '\n')
sources = {}
for pattern in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'apps/**/*.rs', 'apps/**/Cargo.toml', 'crates/**/*.rs', 'crates/**/Cargo.toml', 'adapters/**/*.rs', 'adapters/**/Cargo.toml', 'tools/*.py', 'tools/uv.lock', 'vendor/cpal/src/**/*.rs', 'patches/*.patch', 'vendor/cpal/Cargo.toml', 'vendor/cpal-provenance.json', 'native-dependencies.toml', 'tools/*.sh', 'tools/*.ps1', 'tools/dev', '.github/workflows/*.yml']:
    for file in ROOT.glob(pattern):
        sources[str(file.relative_to(ROOT))] = hashlib.sha256(file.read_bytes()).hexdigest()
report = {'platform': platform.platform(), 'machine': platform.machine(), 'rust': command(['rustc', '-Vv']),
          'git': command(['git', 'rev-parse', 'HEAD']), 'sources_sha256': sources, 'files_sha256': {},
          'build_environment': {name: os.environ.get(name) for name in ['RUSTFLAGS', 'MACOSX_DEPLOYMENT_TARGET', 'CC', 'CFLAGS', 'AR', 'PKG_CONFIG_PATH', 'WindowsSDKVersion', 'VCToolsVersion']},
          'native_compiler': command(['cl.exe'] if sys.platform == 'win32' else ['cc', '--version'])}
if sys.platform == 'darwin':
    report['sdk'] = command(['xcrun', '--show-sdk-version'])
    report['sdk_path'] = command(['xcrun', '--show-sdk-path'])
if sys.platform == 'linux':
    report['native'] = command(['pkg-config', '--modversion', 'libpipewire-0.3', 'alsa'])
for name in ['neonmix-audio', 'neonmix-desktop']:
    binary = name + ('.exe' if sys.platform == 'win32' else '')
    source = ROOT / 'target/release' / binary
    shutil.copy2(source, OUT / binary)
    report['files_sha256'][binary] = hashlib.sha256(source.read_bytes()).hexdigest()
    if sys.platform == 'darwin':
        subprocess.run(['dsymutil', str(source), '-o', str(OUT / (name + '.dSYM'))], check=True)
    elif sys.platform == 'win32':
        pdb = ROOT / 'target/release' / (name.replace('-', '_') + '.pdb')
        if not pdb.exists():
            pdb = ROOT / 'target/release' / (name + '.pdb')
        if not pdb.exists():
            raise RuntimeError(f'Missing debug symbols for {name}')
        shutil.copy2(pdb, OUT / pdb.name)
    else:
        subprocess.run(['objcopy', '--only-keep-debug', str(source), str(OUT / (name + '.debug'))], check=True)
shutil.copytree(ROOT / 'docs', OUT / 'docs', dirs_exist_ok=True)
shutil.copy2(ROOT / 'vendor/cpal/LICENSE', OUT / 'CPAL-LICENSE')
for name in ['Cargo.lock', 'rust-toolchain.toml', 'README.md', 'native-dependencies.toml', '01_NeonMix_设计方案与技术选型.md', '02_NeonMix_完整开发计划.md']:
    shutil.copy2(ROOT / name, OUT / name)
(OUT / 'build.json').write_text(json.dumps(report, indent=2) + '\n')
with zipfile.ZipFile(ROOT / 'artifacts' / (NAME + '.zip'), 'w', zipfile.ZIP_DEFLATED) as archive:
    for file in OUT.rglob('*'):
        if file.is_file():
            archive.write(file, file.relative_to(OUT.parent))
print(OUT)
