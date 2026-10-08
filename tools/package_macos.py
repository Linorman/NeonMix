#!/usr/bin/env python3
"""Build the macOS installer: NeonMix.app (relocated GStreamer runtime) in a DMG.

Run from the repository root through `tools/dev`:

    tools/dev python3 tools/package_macos.py [--skip-build]

The default output is artifacts/installers/; --output selects another project-local
directory and --keep-app retains the verified bundle. The app is ad-hoc signed, not
notarized. Per-user data lives under ~/Library (see packaging/macos/launcher.c).
"""
import argparse
import hashlib
import json
import platform
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

try:
    from .macho_references import relocation_problems
except ImportError:  # Direct tools/dev python3 tools/package_macos.py invocation.
    from macho_references import relocation_problems

ROOT = Path(__file__).resolve().parents[1]
PREFIX = ROOT / '.local/gstreamer/prefix'
PLUGIN_DIR = PREFIX / 'lib/gstreamer-1.0'
SCANNER = PREFIX / 'libexec/gstreamer-1.0/gst-plugin-scanner'
RELEASE = ROOT / 'target/release'
WORKER = ROOT / '.local/airplay/build/neonmix-airplay-worker'
PACKAGING = ROOT / 'packaging/macos'
BINARIES = ['neonmix-desktop', 'neonmix-background', 'neonmix-hub', 'neonmix-audio',
            'neonmix-airplay-profile', 'neonmix-guardian']
MEDIA_PLUGINS = ['coreelements', 'app', 'audioconvert', 'audioresample', 'libav', 'opus',
                 'rtp', 'rtpmanager', 'dtls', 'srtp']
AIRPLAY_PLUGINS = ['coreelements', 'app', 'audioconvert', 'audioresample', 'libav']
SYSTEM = ('/usr/lib/', '/System/')


def run(*args, **kwargs):
    kwargs.setdefault('check', True)
    kwargs.setdefault('cwd', ROOT)
    return subprocess.run([str(a) for a in args], **kwargs)


def output(*args):
    return run(*args, capture_output=True, text=True).stdout


def dependencies(binary):
    names = []
    own_id = output('otool', '-D', binary).splitlines()[1:]  # dylibs list themselves first
    for line in output('otool', '-L', binary).splitlines()[1:]:
        name = line.strip().split(' (')[0]
        if name and name not in own_id and not name.startswith(SYSTEM):
            names.append(name)
    return names


def rpaths(binary):
    paths = []
    for command in re.split(r'^Load command \d+\s*$', output('otool', '-l', binary), flags=re.M):
        if not re.search(r'^\s*cmd LC_RPATH\s*$', command, flags=re.M):
            continue
        match = re.search(r'^\s*path (.+) \(offset \d+\)\s*$', command, flags=re.M)
        if not match:
            raise ValueError(f'{binary}: malformed LC_RPATH command')
        paths.append(match.group(1))
    return paths


def set_rpath(binary, rpath):
    for old in rpaths(binary):
        run('install_name_tool', '-delete_rpath', old, binary, capture_output=True)
    run('install_name_tool', '-add_rpath', rpath, binary, capture_output=True)


def thin(path):
    """GStreamer's macOS runtime ships universal binaries; keep arm64 only."""
    archs = output('lipo', '-archs', path).split()
    if archs != ['arm64']:
        if 'arm64' not in archs:
            sys.exit(f'{path} has no arm64 slice: {archs}')
        run('lipo', '-thin', 'arm64', path, '-output', path)


def build_products():
    run('cargo', 'build', '--release', '--workspace')
    run('cmake', '--build', '.local/airplay/build', '--target', 'neonmix-airplay-worker',
        '--parallel', '8')
    for binary in [*(RELEASE / b for b in BINARIES), WORKER]:
        if not binary.is_file():
            sys.exit(f'missing build product: {binary}')


def bundle_libraries(roots, frameworks):
    """Copy every non-system dependency of `roots` into Frameworks, recursively."""
    copied = {}
    queue = list(roots)
    while queue:
        binary = queue.pop()
        for dep in dependencies(binary):
            name = Path(dep).name
            if name in copied or name == Path(binary).name:
                continue
            source = PREFIX / 'lib' / name
            if not source.is_file():
                sys.exit(f'{binary}: cannot resolve {dep} in {PREFIX / "lib"}')
            target = frameworks / name
            shutil.copy2(source.resolve(), target)
            target.chmod(0o755)
            thin(target)
            copied[name] = target
            queue.append(target)
    return copied


def relink(binary, frameworks_rpath, frameworks):
    for dep in dependencies(binary):
        name = Path(dep).name
        if name == Path(binary).name:
            continue
        if (frameworks / name).is_file():
            run('install_name_tool', '-change', dep, f'@rpath/{name}', binary)
    # Binaries without bundled libraries (the Rust UI, the profile tool) need no
    # rpath, and some have no header room for one.
    if dependencies(binary):
        set_rpath(binary, frameworks_rpath)
    else:
        for old in rpaths(binary):
            run('install_name_tool', '-delete_rpath', old, binary, capture_output=True)


def sign(path):
    run('codesign', '--force', '--sign', '-', '--timestamp=none', path, capture_output=True)


def plugin(name):
    path = PLUGIN_DIR / f'libgst{name}.dylib'
    if not path.is_file():
        sys.exit(f'missing GStreamer plugin: {path}')
    return path


def assemble(app, version, build):
    if app.exists():
        shutil.rmtree(app)
    contents = app / 'Contents'
    bin_dir = contents / 'MacOS/bin'
    frameworks = contents / 'Frameworks'
    helpers = contents / 'Helpers'
    resources = contents / 'Resources'
    for directory in (bin_dir, frameworks, helpers, resources / 'plugins',
                      resources / 'airplay/plugins'):
        directory.mkdir(parents=True)

    for name in BINARIES:
        shutil.copy2(RELEASE / name, bin_dir / name)
    shutil.copy2(WORKER, bin_dir / 'neonmix-airplay-worker')
    shutil.copy2(SCANNER, helpers / 'gst-plugin-scanner')
    for name in MEDIA_PLUGINS:
        shutil.copy2(plugin(name).resolve(), resources / 'plugins' / f'libgst{name}.dylib')
    for name in AIRPLAY_PLUGINS:
        shutil.copy2(plugin(name).resolve(), resources / 'airplay/plugins' / f'libgst{name}.dylib')
    for path in contents.rglob('*'):
        if path.is_file():
            path.chmod(0o755)
            thin(path)

    executables = sorted(bin_dir.iterdir()) + [helpers / 'gst-plugin-scanner']
    for binary in executables:
        run('strip', '-S', binary, capture_output=True)  # release builds carry full DWARF
    plugins = sorted((resources / 'plugins').iterdir())
    airplay_plugins = sorted((resources / 'airplay/plugins').iterdir())
    bundled = bundle_libraries([*executables, *plugins, *airplay_plugins], frameworks)

    for binary in executables:
        relink(binary, '@executable_path/../../Frameworks' if binary.parent == bin_dir
               else '@executable_path/../Frameworks', frameworks)
    for binary in plugins:
        run('install_name_tool', '-id', f'@rpath/{binary.name}', binary)
        relink(binary, '@loader_path/../../Frameworks', frameworks)
    for binary in airplay_plugins:
        run('install_name_tool', '-id', f'@rpath/{binary.name}', binary)
        relink(binary, '@loader_path/../../../Frameworks', frameworks)
    for library in frameworks.iterdir():
        run('install_name_tool', '-id', f'@rpath/{library.name}', library)
        relink(library, '@loader_path', frameworks)

    # Launcher: the only Contents/MacOS entry point.
    launcher = contents / 'MacOS/NeonMix'
    run('clang', '-O2', '-Wall', '-Werror', '-arch', 'arm64', '-mmacosx-version-min=14.6',
        PACKAGING / 'launcher.c', '-o', launcher)
    plist = (PACKAGING / 'Info.plist.in').read_text()
    (contents / 'Info.plist').write_text(
        plist.replace('@VERSION@', version).replace('@BUILD@', build))

    # Inside-out ad-hoc signing: every Mach-O first, the bundle last.
    for path in [*frameworks.iterdir(), *plugins, *airplay_plugins, *executables, launcher]:
        sign(path)
    run('codesign', '--force', '--sign', '-', '--timestamp=none', app, capture_output=True)
    run('codesign', '--verify', '--deep', '--strict', app)
    return bundled


def verify_relocation(app):
    problems = relocation_problems(app)
    if problems:
        sys.exit('non-relocatable references:\n' + '\n'.join(problems))


def make_dmg(app, dmg, volume):
    stage = dmg.parent / 'dmg-stage'
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    run('ditto', app, stage / app.name)
    (stage / 'Applications').symlink_to('/Applications')
    dmg.unlink(missing_ok=True)
    run('hdiutil', 'create', '-volname', volume, '-srcfolder', stage, '-ov', '-format', 'UDZO',
        '-fs', 'HFS+', dmg, capture_output=True)
    shutil.rmtree(stage)


def sha256(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b''):
            digest.update(chunk)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--skip-build', action='store_true', help='reuse existing release builds')
    parser.add_argument('--output', type=Path, default=Path('artifacts/installers'),
                        help='project-local output directory (default: artifacts/installers)')
    parser.add_argument('--keep-app', action='store_true',
                        help='retain the verified .app alongside the DMG')
    args = parser.parse_args()
    out = (args.output if args.output.is_absolute() else ROOT / args.output).resolve()
    if not out.is_relative_to(ROOT):
        parser.error('--output must remain inside the project directory')
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':
        sys.exit('macOS installer must be built on Apple Silicon macOS')
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    commit = output('git', 'rev-parse', '--short=9', 'HEAD').strip()
    if not args.skip_build:
        build_products()
    out.mkdir(parents=True, exist_ok=True)
    app = out / 'NeonMix.app'
    bundled = assemble(app, version, commit)
    verify_relocation(app)
    dmg = out / f'NeonMix-{version}-macos-arm64.dmg'
    make_dmg(app, dmg, f'NeonMix {version}')
    if not args.keep_app:
        shutil.rmtree(app)
    report = {
        'version': version,
        'commit': commit,
        'dmg': dmg.name,
        'dmg_bytes': dmg.stat().st_size,
        'dmg_sha256': sha256(dmg),
        'bundled_libraries': len(bundled),
        'signing': 'ad-hoc, not notarized',
        'retained_app': app.name if args.keep_app else None,
    }
    (out / f'NeonMix-{version}-macos-arm64.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
