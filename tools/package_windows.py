#!/usr/bin/env python3
"""Stage the Windows x64 install tree and build the NSIS installer.

Runs on Windows after `cargo build --release --workspace` (MSVC GStreamer from
tools/prepare_windows_gstreamer.py) and the MinGW AirPlay worker build. See
.github/workflows/windows-installer.yml for the full sequence.

    python tools/package_windows.py --mingw C:/msys64/mingw64 --worker PATH/neonmix-airplay-worker.exe
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

import pefile

ROOT = Path(__file__).resolve().parents[1]
RELEASE = ROOT / 'target/release'
PACKAGING = ROOT / 'packaging/windows'
BINARIES = ['neonmix-desktop', 'neonmix-background', 'neonmix-hub', 'neonmix-audio',
            'neonmix-airplay-profile']
MEDIA_PLUGINS = ['coreelements', 'app', 'audioconvert', 'audioresample', 'libav', 'opus',
                 'rtp', 'rtpmanager', 'dtls', 'srtp']
AIRPLAY_PLUGINS = ['coreelements', 'app', 'audioconvert', 'audioresample', 'libav']
# Not guaranteed on a clean Windows install: ship them app-local.
CRT = {'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'msvcp140_1.dll',
       'msvcp140_2.dll', 'concrt140.dll'}
SYSTEM32 = Path(os.environ.get('SystemRoot', 'C:/Windows')) / 'System32'


def imports(path):
    pe = pefile.PE(str(path), fast_load=True)
    pe.parse_data_directories(directories=[
        pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_IMPORT'],
        pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT']])
    names = []
    for table in ('DIRECTORY_ENTRY_IMPORT', 'DIRECTORY_ENTRY_DELAY_IMPORT'):
        names += [entry.dll.decode() for entry in getattr(pe, table, [])]
    pe.close()
    return names


def index(search):
    """Case-insensitive name -> path over `search`; earlier directories win."""
    files = {}
    for directory in reversed(search):
        if directory.is_dir():
            files.update({p.name.lower(): p for p in directory.iterdir() if p.is_file()})
    return files


def collect(roots, search, destination, owner):
    """Copy the non-system DLL closure of `roots` into `destination`."""
    queue = list(roots)
    seen = set()
    available = index(search)
    while queue:
        current = queue.pop()
        for name in imports(current):
            lowered = name.lower()
            if lowered in seen or lowered.startswith(('api-ms-win-', 'ext-ms-win-')):
                continue
            seen.add(lowered)
            source = available.get(lowered)
            if source is None:
                if lowered in CRT and (SYSTEM32 / name).is_file():
                    source = SYSTEM32 / name
                elif (SYSTEM32 / name).is_file():
                    continue  # Windows component
                else:
                    sys.exit(f'{current.name}: cannot resolve {name}')
            target = destination / source.name
            if target.exists():
                if sha256(target) != sha256(source):
                    sys.exit(f'DLL name collision with different content: {source.name} '
                             f'({owner[source.name]} vs {current})')
            else:
                shutil.copy2(source, target)
                owner[source.name] = current
                queue.append(target)


def sha256(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b''):
            digest.update(chunk)
    return digest.hexdigest()


def msvc_prefix():
    lock = json.loads((ROOT / '.local/gstreamer-windows/lock.json').read_text(encoding='utf-8'))
    return Path(lock['prefix'])


def stage(root, mingw, worker):
    msvc = msvc_prefix()
    if root.exists():
        shutil.rmtree(root)
    bin_dir = root / 'bin'
    for directory in (bin_dir, root / 'plugins', root / 'airplay/plugins', root / 'licenses'):
        directory.mkdir(parents=True)
    for name in BINARIES:
        shutil.copy2(RELEASE / f'{name}.exe', bin_dir / f'{name}.exe')
    shutil.copy2(worker, bin_dir / 'neonmix-airplay-worker.exe')
    for name in MEDIA_PLUGINS:
        source = msvc / 'lib/gstreamer-1.0' / f'gst{name}.dll'
        if not source.is_file():
            sys.exit(f'missing MSVC plugin {source}')
        shutil.copy2(source, root / 'plugins' / source.name)
    for name in AIRPLAY_PLUGINS:
        source = mingw / 'lib/gstreamer-1.0' / f'libgst{name}.dll'
        if not source.is_file():
            sys.exit(f'missing MinGW plugin {source}')
        shutil.copy2(source, root / 'airplay/plugins' / source.name)

    owner = {}
    msvc_roots = [bin_dir / f'{n}.exe' for n in BINARIES] + sorted((root / 'plugins').iterdir())
    mingw_roots = [bin_dir / 'neonmix-airplay-worker.exe'] + sorted((root / 'airplay/plugins').iterdir())
    collect(msvc_roots, [msvc / 'bin', msvc / 'lib'], bin_dir, owner)
    collect(mingw_roots, [mingw / 'bin'], bin_dir, owner)

    # Launcher: GUI-subsystem, no console window.
    subprocess.run(['cl', '/nologo', '/O2', '/W3', '/WX', '/utf-8', f'/Fe:{root / "NeonMix.exe"}',
                    f'/Fo:{root.parent / "launcher.obj"}', str(PACKAGING / 'launcher.c'),
                    '/link', '/SUBSYSTEM:WINDOWS', 'user32.lib', 'shell32.lib'], check=True)
    return owner


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--mingw', type=Path, required=True, help='MSYS2 mingw64 prefix')
    parser.add_argument('--worker', type=Path, required=True, help='MinGW neonmix-airplay-worker.exe')
    parser.add_argument('--makensis', default='makensis')
    args = parser.parse_args()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version']
    commit = subprocess.run(['git', 'rev-parse', '--short=9', 'HEAD'], cwd=ROOT, check=True,
                            capture_output=True, text=True).stdout.strip()
    out = ROOT / 'artifacts/installers'
    out.mkdir(parents=True, exist_ok=True)
    root = out / 'NeonMix'
    stage(root, args.mingw, args.worker)
    installer = out / f'NeonMix-{version}-windows-x64-setup.exe'
    installer.unlink(missing_ok=True)
    subprocess.run([args.makensis, f'/DVERSION={version}', f'/DSOURCE={root}',
                    f'/DOUTFILE={installer}', str(PACKAGING / 'neonmix.nsi')], check=True)
    report = {
        'version': version,
        'commit': commit,
        'installer': installer.name,
        'installer_bytes': installer.stat().st_size,
        'installer_sha256': sha256(installer),
        'signing': 'unsigned',
        'stage_files': sum(1 for p in root.rglob('*') if p.is_file()),
    }
    (out / f'NeonMix-{version}-windows-x64.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
