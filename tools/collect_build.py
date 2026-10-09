#!/usr/bin/env python3
"""Preserve native binaries, debug symbols, dependency licenses and source hashes."""
import hashlib
import argparse
import json
import os
import platform
import re
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

from archive_policy import assert_clean, copy_ignore, excluded

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--native-media', action='store_true')
args = parser.parse_args()
native_media = sys.platform == 'darwin' or args.native_media
NAME = f'neonmix-{"e07" if native_media else "e01"}-{sys.platform}-{platform.machine()}'
OUT = ROOT / 'artifacts' / NAME
OUT.mkdir(parents=True, exist_ok=True)


def command(args):
    try:
        result = subprocess.run(args, cwd=ROOT, capture_output=True, text=True)
    except OSError as error:
        return {'command': args, 'exit_code': None, 'stdout': '', 'stderr': str(error)}
    return {'command': args, 'exit_code': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}


def macos_symbols(binary, destination):
    # rustc sometimes records bundled native objects under a temporary rlib
    # path removed after linking. Reuse the actual retained archive while
    # dsymutil resolves its members; never create substitute object contents.
    inspected = command(['dsymutil', '--dump-debug-map', str(binary)])
    base = ROOT / 'target/release/deps'
    pattern = re.escape(str(base)) + r'/rustc[^/\s]+/[^/\s]+\.rlib'
    links, directories = [], []
    try:
        for name in sorted(set(re.findall(pattern, inspected['stderr']))):
            missing = Path(name)
            archive = base / missing.name
            if missing.exists() or not archive.is_file():
                continue
            if not missing.parent.exists():
                missing.parent.mkdir()
                directories.append(missing.parent)
            missing.symlink_to(archive)
            links.append(missing)
        result = command(['dsymutil', str(binary), '-o', str(destination)])
        if result['exit_code']:
            raise RuntimeError(result['stderr'])
        if result['stderr']:
            print(result['stderr'], file=sys.stderr)
    finally:
        for link in links:
            link.unlink(missing_ok=True)
        for directory in directories:
            directory.rmdir()


metadata = command(['cargo', 'metadata', '--locked', '--format-version', '1'])
if metadata['exit_code']:
    raise RuntimeError(metadata['stderr'])
metadata = json.loads(metadata['stdout'])
(OUT / 'cargo-metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
licenses = [{'name': p['name'], 'version': p['version'], 'license': p['license'], 'source': p['source']} for p in metadata['packages']]
(OUT / 'dependency-licenses.json').write_text(json.dumps(licenses, indent=2) + '\n')
sources = {}
for pattern in ['drivers/windows/**/*.cpp', 'drivers/windows/**/*.h', 'drivers/windows/**/*.rc',
                'drivers/windows/**/*.inx', 'drivers/windows/**/*.vcxproj', 'drivers/windows/**/*.props',
                'drivers/windows/**/*.json', 'drivers/windows/**/LICENSE-*', 'drivers/linux/*.service']:
    for file in ROOT.glob(pattern):
        sources[str(file.relative_to(ROOT))] = hashlib.sha256(file.read_bytes()).hexdigest()
for pattern in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'apps/**/*.rs', 'apps/**/Cargo.toml', 'crates/**/*.rs', 'crates/**/Cargo.toml', 'adapters/**/*.rs', 'adapters/**/Cargo.toml', 'drivers/macos/hal/**/*.rs', 'drivers/macos/hal/Cargo.toml', 'drivers/macos/Info.plist', 'tools/*.py', 'tools/uv.lock', 'vendor/cpal/src/**/*.rs', 'vendor/tympan-aspl/src/**/*.rs', 'vendor/tympan-aspl/Cargo.toml', 'vendor/tympan-aspl/LICENSE-*', 'patches/*.patch', 'patches/*.md', 'vendor/cpal/Cargo.toml', 'vendor/cpal-provenance.json', 'native-dependencies.toml', 'tools/*.sh', 'tools/*.ps1', 'tools/dev', '.github/workflows/*.yml']:
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
if sys.platform == 'darwin':
    report['gstreamer'] = command([str(ROOT / 'target/release/neonmix-hub'), 'runtime'])
    report['gstreamer_archive'] = json.loads((ROOT / '.local/gstreamer/lock.json').read_text())
    hal_bundle = ROOT / 'artifacts/macos/NeonMixHAL.driver'
    if hal_bundle.exists():
        shutil.copytree(hal_bundle, OUT / hal_bundle.name, dirs_exist_ok=True)
        hal_binary = hal_bundle / 'Contents/MacOS/NeonMixHAL'
        report['files_sha256']['NeonMixHAL.driver/Contents/MacOS/NeonMixHAL'] = hashlib.sha256(hal_binary.read_bytes()).hexdigest()
        macos_symbols(hal_binary, OUT / 'NeonMixHAL.dSYM')
if sys.platform == 'win32' and native_media:
    report['gstreamer_archive'] = json.loads((ROOT / '.local/gstreamer-windows/lock.json').read_text())
    gst_prefix = Path(report['gstreamer_archive']['prefix'])
    # A Windows archive must include the native DLLs and plugins needed by Hub.
    for directory in ['bin', 'lib/gstreamer-1.0', 'libexec', 'share/licenses']:
        source = gst_prefix / directory
        if source.exists():
            shutil.copytree(source, OUT / 'gstreamer' / directory, dirs_exist_ok=True)
    (OUT / 'run.cmd').write_text('@echo off\r\nset "PATH=%~dp0gstreamer\\bin;%PATH%"\r\nset "GST_PLUGIN_SYSTEM_PATH_1_0=%~dp0gstreamer\\lib\\gstreamer-1.0"\r\n"%~dp0neonmix-desktop.exe" %*\r\n', encoding='utf-8')
for name in ['neonmix-audio', 'neonmix-desktop', 'neonmix-background'] + (['neonmix-guardian'] if sys.platform in ['darwin', 'linux'] else []) + (['neonmix-hub'] if native_media else []):
    binary = name + ('.exe' if sys.platform == 'win32' else '')
    source = ROOT / 'target/release' / binary
    shutil.copy2(source, OUT / binary)
    report['files_sha256'][binary] = hashlib.sha256(source.read_bytes()).hexdigest()
    if sys.platform == 'darwin':
        macos_symbols(source, OUT / (name + '.dSYM'))
    elif sys.platform == 'win32':
        pdb = ROOT / 'target/release' / (name.replace('-', '_') + '.pdb')
        if not pdb.exists():
            pdb = ROOT / 'target/release' / (name + '.pdb')
        if not pdb.exists():
            raise RuntimeError(f'Missing debug symbols for {name}')
        shutil.copy2(pdb, OUT / pdb.name)
    else:
        subprocess.run(['objcopy', '--only-keep-debug', str(source), str(OUT / (name + '.debug'))], check=True)
shutil.copytree(ROOT / 'docs', OUT / 'docs', dirs_exist_ok=True, ignore=copy_ignore)
shutil.copy2(ROOT / 'vendor/cpal/LICENSE', OUT / 'CPAL-LICENSE')
shutil.copy2(ROOT / 'vendor/tympan-aspl/LICENSE-MIT', OUT / 'TYMPAN-LICENSE-MIT')
shutil.copy2(ROOT / 'vendor/tympan-aspl/LICENSE-APACHE', OUT / 'TYMPAN-LICENSE-APACHE')
for name in ['Cargo.lock', 'rust-toolchain.toml', 'README.md', 'README.zh-CN.md', 'native-dependencies.toml']:
    shutil.copy2(ROOT / name, OUT / name)
(OUT / 'build.json').write_text(json.dumps(report, indent=2) + '\n')
assert_clean(OUT)
with zipfile.ZipFile(ROOT / 'artifacts' / (NAME + '.zip'), 'w', zipfile.ZIP_DEFLATED) as archive:
    for file in OUT.rglob('*'):
        if file.is_file() and not excluded(file.relative_to(OUT)):
            archive.write(file, file.relative_to(OUT.parent))
print(OUT)
