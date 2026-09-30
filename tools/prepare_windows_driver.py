#!/usr/bin/env python3
"""Restore locked Microsoft WDK/SDK packages into this project; never install them globally."""
import hashlib
import json
from pathlib import Path
import stat
import zipfile
from downloads import download

ROOT = Path(__file__).resolve().parents[1]
lock = json.loads((ROOT / 'drivers/windows/wavert/sdk-lock.json').read_text())
for record in lock['packages']:
    archive = ROOT / '.local/windows-driver-sdk' / record['url'].rsplit('/', 1)[1]
    download(record['url'], archive, record['sha256'], max_time=900)
    destination = (ROOT / record['directory']).resolve()
    if not destination.is_relative_to(ROOT):
        raise ValueError('SDK destination must stay inside the project')
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as package:
        for entry in package.infolist():
            target = (destination / entry.filename).resolve()
            if not target.is_relative_to(destination) or stat.S_ISLNK(entry.external_attr >> 16):
                raise ValueError('unsafe SDK archive entry')
        package.extractall(destination)
    print(record['name'], record['version'], hashlib.sha256(archive.read_bytes()).hexdigest())
