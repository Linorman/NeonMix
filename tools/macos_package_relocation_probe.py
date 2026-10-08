#!/usr/bin/env python3
"""Relocate a finished app and validate bundled payloads/media with no development runtime paths."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from macho_references import relocation_problems

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    source, output = args.app.resolve(), args.output.resolve()
    if not all(path.is_relative_to(ROOT) for path in [source, output]):
        parser.error('all files must stay inside the project')
    output.mkdir(parents=True, exist_ok=True)
    lab = Path(tempfile.mkdtemp(prefix='p10-relocated-', dir=ROOT / '.local/tmp'))
    app = lab / '另一目录 With Spaces' / 'NeonMix.app'
    report = {'passed': False, 'scope': 'finished app relocation/payload startup/bundled DTLS media; GUI/installation deferred', 'checks': {}}
    try:
        shutil.copytree(source, app)
        assert not relocation_problems(app)
        contents = app / 'Contents'
        binaries = contents / 'MacOS/bin'
        environment = dict(os.environ)
        for key in list(environment):
            if key.startswith(('DYLD_', 'GST_', 'NEONMIX_')):
                environment.pop(key)
        environment.update(PATH='/usr/bin:/bin:/usr/sbin:/sbin', TMPDIR=str(lab), TMP=str(lab), TEMP=str(lab),
            XDG_CACHE_HOME=str(lab / 'cache'), GST_REGISTRY=str(lab / 'registry.bin'),
            GST_PLUGIN_SYSTEM_PATH_1_0=str(contents / 'Resources/plugins'), GST_PLUGIN_PATH_1_0='', GST_PLUGIN_PATH='',
            GST_PLUGIN_SCANNER=str(contents / 'Helpers/gst-plugin-scanner'),
            NEONMIX_AIRPLAY_RUNTIME=str(contents / 'Resources/airplay'), DYLD_PRINT_LIBRARIES='1')
        for name in ['neonmix-desktop', 'neonmix-background', 'neonmix-hub', 'neonmix-audio', 'neonmix-guardian']:
            result = subprocess.run([str(binaries / name), '--help'], env=environment, capture_output=True, timeout=20)
            (output / (name + '.log')).write_bytes(result.stdout + result.stderr)
            assert result.returncode == 0, name
            assert str(ROOT / '.local/gstreamer').encode() not in result.stderr
            report['checks'][name + '_payload_starts'] = True
        for name, arguments in [('runtime', ['runtime']), ('media', ['probe', '--seconds', '2', '--streams', '2'])]:
            result = subprocess.run([str(binaries / 'neonmix-hub'), *arguments], env=environment, capture_output=True, timeout=30)
            (output / (name + '.log')).write_bytes(result.stdout + result.stderr)
            assert result.returncode == 0, name
            assert str(ROOT / '.local/gstreamer').encode() not in result.stderr
            report['checks'][name + '_bundled_runtime_only'] = True
        report['payload_sha256'] = {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in binaries.iterdir() if path.is_file()}
        report['passed'] = True
    except Exception as error:
        report['error'] = str(error)
    finally:
        shutil.rmtree(lab)
        report['temporary_bundle_removed'] = not lab.exists()
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
