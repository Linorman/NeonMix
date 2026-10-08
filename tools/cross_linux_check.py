#!/usr/bin/env python3
"""Read-only cross type-check using Ubuntu headers; NOT a Linux runtime test.
All archives, headers and Rust target libraries stay in .local/. No host package installation.
"""
import gzip
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / '.local' / 'cross-linux'
CACHE.mkdir(parents=True, exist_ok=True)
SYSROOT = CACHE / 'sysroot'
SYSROOT.mkdir(exist_ok=True)
RUSTROOT = CACHE / 'rust'
RUSTROOT.mkdir(exist_ok=True)


from downloads import download


rust_name = 'rust-std-1.95.0-x86_64-unknown-linux-gnu.tar.xz'
rust_url = 'https://static.rust-lang.org/dist/2026-04-16/' + rust_name
sha_file = CACHE / (rust_name + '.sha256')
download(rust_url + '.sha256', sha_file)
rust_sha = download(rust_url, CACHE / rust_name, sha_file.read_text().split()[0])
with tarfile.open(CACHE / rust_name) as tar:
    tar.extractall(CACHE, filter='data')
source = CACHE / rust_name.removesuffix('.tar.xz') / 'rust-std-x86_64-unknown-linux-gnu' / 'lib' / 'rustlib' / 'x86_64-unknown-linux-gnu'
target = RUSTROOT / 'lib' / 'rustlib' / 'x86_64-unknown-linux-gnu'
target.parent.mkdir(parents=True, exist_ok=True)
if not target.exists():
    target.symlink_to(source, target_is_directory=True)

wanted = {'libpipewire-0.3-dev', 'libspa-0.2-dev', 'libasound2-dev', 'libc6-dev', 'linux-libc-dev', 'libdbus-1-dev'}
build_audio = '--build-audio' in sys.argv
if build_audio:
    wanted |= {'libpipewire-0.3-0t64', 'libasound2t64', 'libc6', 'libgcc-s1', 'libgcc-13-dev', 'libdbus-1-3'}
packages = {}
base = 'https://archive.ubuntu.com/ubuntu/'
for component in ['main', 'universe']:
    index = CACHE / f'{component}-Packages.gz'
    download(base + f'dists/noble/{component}/binary-amd64/Packages.gz', index)
    for paragraph in gzip.decompress(index.read_bytes()).decode().split('\n\n'):
        fields = dict(line.split(': ', 1) for line in paragraph.splitlines() if ': ' in line and not line.startswith(' '))
        if fields.get('Package') in wanted:
            packages[fields['Package']] = fields
    if wanted <= packages.keys():
        break
if not wanted <= packages.keys():
    raise RuntimeError(f'Missing packages: {wanted - packages.keys()}')
for name, fields in packages.items():
    deb = CACHE / Path(fields['Filename']).name
    download(base + fields['Filename'], deb, fields['SHA256'])
    members = subprocess.check_output(['ar', 't', str(deb)], text=True).splitlines()
    member = next(m for m in members if m.startswith('data.tar'))
    archive = CACHE / member
    with archive.open('wb') as out:
        subprocess.run(['ar', 'p', str(deb), member], stdout=out, check=True)
    subprocess.run(['tar', '-xf', str(archive), '-C', str(SYSROOT)], check=True)
    archive.unlink()

# Ubuntu's packages assume a merged /usr filesystem; recreate its relative links.
for alias in ['lib', 'lib64']:
    link = SYSROOT / alias
    if not link.exists() and not link.is_symlink():
        link.symlink_to('usr/' + alias, target_is_directory=True)

env = os.environ.copy()
env['CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS'] = f'--sysroot={RUSTROOT}'
env['PKG_CONFIG_ALLOW_CROSS'] = '1'
env['PKG_CONFIG_SYSROOT_DIR'] = str(SYSROOT)
env['PKG_CONFIG_PATH'] = str(SYSROOT / 'usr/lib/x86_64-linux-gnu/pkgconfig')
env['PKG_CONFIG_LIBDIR'] = env['PKG_CONFIG_PATH']
env['BINDGEN_EXTRA_CLANG_ARGS'] = f'--target=x86_64-linux-gnu --sysroot={SYSROOT} -I{SYSROOT}/usr/include/x86_64-linux-gnu'
env['CC_x86_64_unknown_linux_gnu'] = 'clang'
env['CFLAGS_x86_64_unknown_linux_gnu'] = env['BINDGEN_EXTRA_CLANG_ARGS']
# Apple's ar/ranlib silently omit ELF objects. Use the pinned Rust LLVM archive tool.
rustc_path = Path(subprocess.check_output(['rustup', 'which', '--toolchain', '1.95.0', 'rustc'], text=True).strip())
manifest = tomllib.loads((rustc_path.parent.parent / 'lib/rustlib/multirust-channel-manifest.toml').read_text())
llvm_package = next(key for key in manifest['pkg'] if key.startswith('llvm-tools'))
llvm_info = manifest['pkg'][llvm_package]['target']['aarch64-apple-darwin']
llvm_root = CACHE / 'llvm-tools'
llvm_root.mkdir(exist_ok=True)
llvm_archive = llvm_root / Path(llvm_info['xz_url']).name
download(llvm_info['xz_url'], llvm_archive, llvm_info['xz_hash'])
if not list(llvm_root.rglob('llvm-ar')):
    with tarfile.open(llvm_archive) as archive:
        archive.extractall(llvm_root, filter='data')
env['AR_x86_64_unknown_linux_gnu'] = str(next(llvm_root.rglob('llvm-ar')))
env['LIBCLANG_PATH'] = '/Library/Developer/CommandLineTools/usr/lib'
if build_audio:
    clang = subprocess.check_output(['xcrun', '--find', 'clang'], text=True).strip()
    rustc = Path(subprocess.check_output(['rustup', 'which', '--toolchain', '1.95.0', 'rustc'], text=True).strip())
    rust_lld = rustc.parent.parent / 'lib/rustlib/aarch64-apple-darwin/bin/rust-lld'
    lld = CACHE / 'ld.lld'
    if not lld.exists():
        lld.symlink_to(rust_lld)
    linker = CACHE / 'linux-linker'
    import shlex
    linker.write_text('#!/bin/sh\nexec ' + ' '.join(shlex.quote(str(arg)) for arg in [clang,
        '--target=x86_64-linux-gnu', f'--sysroot={SYSROOT}', f'-fuse-ld={lld}',
        f'-B{SYSROOT}/usr/lib/gcc/x86_64-linux-gnu/13',
        f'-L{SYSROOT}/usr/lib/gcc/x86_64-linux-gnu/13',
        f'-Wl,-rpath-link,{SYSROOT}/usr/lib/x86_64-linux-gnu']) + ' "$@"\n')
    linker.chmod(0o755)
    env['CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER'] = str(linker)
report = {'rust_std_sha256': rust_sha, 'packages': {name: {k: v[k] for k in ['Version', 'Filename', 'SHA256']} for name, v in packages.items()}, 'type': 'cross_link' if build_audio else 'cross_check_only'}
(ROOT / ('artifacts/linux-link-dependencies.json' if build_audio else 'artifacts/linux-cross-dependencies.json')).write_text(json.dumps(report, indent=2) + '\n')
subprocess.run(['cargo', *(['build', '--release'] if build_audio else ['check']), '--locked', *(['--workspace'] if '--workspace' in sys.argv else ['-p', 'neonmix-audio']), '--target', 'x86_64-unknown-linux-gnu'], env=env, check=True, cwd=ROOT)

if build_audio and '--test-binaries' in sys.argv:
    result = subprocess.run(['cargo', 'test', '-p', 'neonmix-core', '-p', 'neonmix-io', '--release', '--locked', '--target',
                             'x86_64-unknown-linux-gnu', '--no-run', '--message-format=json-render-diagnostics'],
                            cwd=ROOT, env=env, capture_output=True, text=True)
    print(result.stderr)
    result.check_returncode()
    test_binaries = []
    for line in result.stdout.splitlines():
        row = json.loads(line)
        if row.get('reason') == 'compiler-artifact' and row.get('executable'):
            test_binaries.append(row['executable'])
    (ROOT / 'artifacts/linux-test-binaries.json').write_text(json.dumps(test_binaries, indent=2) + '\n')

if build_audio:
    import hashlib
    source_hashes = {}
    for glob in ['Cargo.toml', 'Cargo.lock', 'crates/**/*.rs', 'apps/**/*.rs', 'adapters/**/*.rs', 'vendor/cpal/src/**/*.rs']:
        for source in ROOT.glob(glob):
            source_hashes[str(source.relative_to(ROOT))] = hashlib.sha256(source.read_bytes()).hexdigest()
    executable = ROOT / 'target/x86_64-unknown-linux-gnu/release/neonmix-audio'
    (ROOT / 'artifacts/linux-build.json').write_text(json.dumps({
        'target': 'x86_64-unknown-linux-gnu', 'binary': str(executable.relative_to(ROOT)),
        'binary_sha256': hashlib.sha256(executable.read_bytes()).hexdigest(),
        'source_hashes': source_hashes,
    }, indent=2) + '\n')
