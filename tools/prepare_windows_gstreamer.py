#!/usr/bin/env python3
"""Extract checksum-locked official MSVC packages without installing on Windows."""
import json
from pathlib import Path
import subprocess
import sys

from downloads import download

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / '.local/gstreamer-windows'
VERSION = '1.26.10'
PACKAGES = {
    'runtime': ('gstreamer-1.0-msvc-x86_64-1.26.10.msi', 'a863bf3faa49e9f33bd3cc42967b473482d4dc98655ed95cba1ac59f26fb0cfb'),
    'devel': ('gstreamer-1.0-devel-msvc-x86_64-1.26.10.msi', '8487a115fea3b0b0b4c55a9a413ad1adab0fe78f6681feedc60858a48613a121'),
}


def main():
    if sys.platform != 'win32':
        raise SystemExit('This entry requires Windows x64/MSVC.')
    records = []
    for kind, (name, digest) in PACKAGES.items():
        url = f'https://gstreamer.freedesktop.org/data/pkg/windows/{VERSION}/msvc/{name}'
        archive = BASE / 'downloads' / name
        download(url, archive, digest, max_time=900)
        destination = BASE / 'extracted'
        destination.mkdir(parents=True, exist_ok=True)
        log = ROOT / 'artifacts/windows' / f'gstreamer-{kind}-extract.log'
        log.parent.mkdir(parents=True, exist_ok=True)
        # Administrative extraction copies files; it does not register/install a product.
        subprocess.run(['msiexec.exe', '/a', str(archive), '/qn',
                        f'TARGETDIR={destination}', '/L*v', str(log)], check=True, timeout=600)
        records.append({'url': url, 'sha256': digest})
    candidates = list((BASE / 'extracted').rglob('gst-launch-1.0.exe'))
    if len(candidates) != 1:
        raise RuntimeError(f'Expected one MSVC runtime, found {candidates}')
    prefix = candidates[0].parent.parent
    if not (prefix / 'bin/pkg-config.exe').exists():
        raise RuntimeError('Official development package did not provide pkg-config.exe')
    report = {'version': VERSION, 'prefix': str(prefix), 'packages': records,
              'installation': 'project-local administrative extraction only'}
    (BASE / 'lock.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    (ROOT / 'artifacts/windows/gstreamer-lock.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
