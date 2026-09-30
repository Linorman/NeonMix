#!/usr/bin/env python3
"""Extract official macOS GStreamer runtime locally; no system installation.

Rust sys crates ship FFI declarations and only need link metadata, not the
C development SDK. Exact downloaded archive hash is checked upstream.
"""
import concurrent.futures
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
VERSION = '1.28.7'
GLOBAL = ROOT / '.local/gstreamer'
BASE = GLOBAL / 'versions' / VERSION
URL = f'https://gstreamer.freedesktop.org/data/pkg/osx/{VERSION}/gstreamer-1.0-{VERSION}-universal.pkg'


def main():
    if sys.platform != 'darwin':
        raise SystemExit('This extraction entry is macOS only.')
    downloads = BASE / 'downloads'
    downloads.mkdir(parents=True, exist_ok=True)
    expected = '529fdf4a4027d942e59b5b3564f6400adaa008f63ce5f3fed4ffe35d73911994'
    size = 153594157
    archive = downloads / 'runtime.pkg'
    valid = archive.exists() and hashlib.sha256(archive.read_bytes()).hexdigest() == expected
    if not valid:
        chunk = 4 * 1024 * 1024
        def get_part(index):
            start = index * chunk
            end = min(size, start + chunk) - 1
            path = downloads / f'part-{index:03}'
            if path.exists() and path.stat().st_size == end - start + 1:
                return path
            for attempt in range(5):
                try:
                    request = urllib.request.Request(URL + f'?neonmix-part={index}', headers={'Range': f'bytes={start}-{end}'})
                    with urllib.request.urlopen(request, timeout=40) as response:
                        data = response.read()
                    if len(data) != end - start + 1:
                        raise ValueError('unexpected range size')
                    path.write_bytes(data)
                    print(f'part {index} complete', flush=True)
                    return path
                except Exception:
                    if attempt == 4:
                        raise
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            parts = list(pool.map(get_part, range((size + chunk - 1) // chunk)))
        with archive.open('wb') as out:
            for part in parts:
                out.write(part.read_bytes())
        if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
            raise SystemExit('GStreamer archive hash mismatch')
        for part in parts:
            part.unlink()
    expanded = BASE / 'expanded'
    prefix = BASE / 'prefix'
    prefix.mkdir(exist_ok=True)
    marker = prefix / '.neonmix-extracted'
    if not marker.exists() or marker.read_text() != expected + ':layout2':
        if not expanded.exists():
            subprocess.run(['pkgutil', '--expand-full', str(archive), str(expanded)], check=True)
        for payload in expanded.glob('*.pkg/Payload'):
            if payload.parent.name.startswith('osx-framework'):
                continue
            for source in payload.rglob('*'):
                destination = prefix / source.relative_to(payload)
                if source.is_symlink():
                    if destination.is_symlink() or destination.exists():
                        destination.unlink()
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.symlink_to(source.readlink())
                elif source.is_dir():
                    destination.mkdir(parents=True, exist_ok=True)
                else:
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, destination)
        marker.write_text(expected + ':layout2')
    if expanded.exists():
        shutil.rmtree(expanded)
    link = prefix
    pc = BASE / 'pkgconfig'
    pc.mkdir(exist_ok=True)
    for package, library in [('glib-2.0', 'glib-2.0'), ('gobject-2.0', 'gobject-2.0'), ('gio-2.0', 'gio-2.0'), ('gstreamer-1.0', 'gstreamer-1.0'), ('gstreamer-base-1.0', 'gstbase-1.0'), ('gstreamer-app-1.0', 'gstapp-1.0'), ('gstreamer-rtp-1.0', 'gstrtp-1.0')]:
        version = '2.82.4' if package in ('glib-2.0', 'gobject-2.0', 'gio-2.0') else VERSION
        (pc / f'{package}.pc').write_text(f'prefix={link}\nlibdir=${{prefix}}/lib\nName: {package}\nDescription: Official runtime FFI link metadata\nVersion: {version}\nLibs: -L${{libdir}} -l{library}\n')
    plugins = BASE / 'plugins'
    plugins.mkdir(exist_ok=True)
    for name in ('coreelements', 'app', 'opus', 'rtp', 'rtpmanager', 'dtls', 'srtp', 'audioconvert', 'audioresample', 'typefindfunctions'):
        selected = plugins / f'libgst{name}.dylib'
        if not selected.exists():
            selected.symlink_to(prefix / 'lib/gstreamer-1.0' / selected.name)
    for name in ('prefix', 'pkgconfig', 'plugins'):
        alias = GLOBAL / name
        if alias.is_symlink():
            alias.unlink()
        elif alias.exists():
            alias.rename(GLOBAL / f'superseded-{name}')
        alias.symlink_to(BASE / name, target_is_directory=True)
    registry = GLOBAL / 'registry.bin'
    if registry.exists():
        registry.unlink()
    # Runtime dylibs contain framework install names. DYLD_LIBRARY_PATH lets
    # dyld resolve the same named official libraries within the extracted tree.
    report = {'version': VERSION, 'source': URL, 'sha256': expected, 'prefix': str(prefix), 'libraries': {'glib': '2.82.4', 'opus': '1.5.2', 'libsrtp': '2.8.0', 'openssl': '3.5.0'}, 'installation': 'project-local extraction; no installer scripts run'}
    (BASE / 'lock.json').write_text(json.dumps(report, indent=2) + '\n')
    (GLOBAL / 'lock.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
