#!/usr/bin/env python3
"""Package only successfully cross-linked code and the exact current Linux test executables."""
import hashlib
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parents[1]
metadata = json.loads((ROOT / 'artifacts/linux-build.json').read_text())
for name, digest in metadata['source_hashes'].items():
    if hashlib.sha256((ROOT / name).read_bytes()).hexdigest() != digest:
        raise RuntimeError(f'Source changed after the Linux build: {name}; rebuild before packaging')
binary = ROOT / metadata['binary']
if hashlib.sha256(binary.read_bytes()).hexdigest() != metadata['binary_sha256']:
    raise RuntimeError('Linux executable hash mismatch')
tests = [Path(p) for p in json.loads((ROOT / 'artifacts/linux-test-binaries.json').read_text())]
manifest = {f'tests/{p.name}': hashlib.sha256(p.read_bytes()).hexdigest() for p in tests}
manifest['target/release/neonmix-audio'] = metadata['binary_sha256']
manifest_path = ROOT / '.local/linux-vm/payload-manifest.json'
manifest_path.write_text(json.dumps(manifest, indent=2)+'\n')
with tarfile.open(ROOT / '.local/linux-vm/payload.tar', 'w') as archive:
    archive.add(binary, arcname='target/release/neonmix-audio')
    for name in ['probe.py', 'linux_runtime_probe.py']:
        archive.add(ROOT / 'tools' / name, arcname='tools/' + name)
    for test in tests:
        archive.add(test, arcname='tests/' + test.name)
    archive.add(manifest_path, arcname='payload-manifest.json')
print(ROOT / '.local/linux-vm/payload.tar')
