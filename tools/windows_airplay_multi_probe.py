#!/usr/bin/env python3
"""Windows multi-entry playback regression with isolated credentials and encrypted PCM.

Run via tools/dev.ps1. Uses the explicitly selected output, no Apple hardware.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import ssl
import subprocess
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen
import uuid

from airplay_multi_source_probe import DigitalSource
from airplay_hub_probe import wait_for

ROOT = Path(__file__).resolve().parents[1]


def powershell(script):
    return subprocess.check_output(['powershell', '-NoProfile', '-Command', script], text=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--hub', type=Path, required=True)
    parser.add_argument('--runtime', type=Path, required=True)
    parser.add_argument('--crypto', type=Path, required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--sources', type=int, choices=[2, 4], default=2)
    parser.add_argument('--native-sources', type=int, choices=[0, 1, 2], default=0)
    parser.add_argument('--codec', choices=['pcm', 'alac'], default='pcm')
    parser.add_argument('--omit-spf', action='store_true')
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    assert os.name == 'nt'
    report_path = args.report.resolve()
    report_path.relative_to(ROOT)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    base = ROOT / '.local/tmp' / ('multi-playback-' + uuid.uuid4().hex)
    base.mkdir(parents=True)
    hub_path = args.hub.resolve()
    package_plugins = hub_path.parent.parent / 'plugins'
    if package_plugins.is_dir():
        os.environ.update(GST_PLUGIN_SYSTEM_PATH_1_0=str(package_plugins), GST_PLUGIN_PATH_1_0='',
                          GST_PLUGIN_PATH='', GST_REGISTRY=str(base / 'gst-registry.bin'), GST_REGISTRY_FORK='no')
    handles = [os.add_dll_directory(str(hub_path.parent))]
    handles += [os.add_dll_directory(p) for p in os.environ['PATH'].split(';') if p and Path(p).is_dir()]
    sources = []
    native = []
    hub = None
    report = dict(passed=False, sources=args.sources, native_sources=args.native_sources, codec=args.codec, omit_spf=args.omit_spf, platform='Windows', stages=[],
                  scope='isolated encrypted digital sources; no Apple interoperability or long-duration claim',
                  binary_sha256={p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                                 for p in [hub_path, args.runtime.resolve() / 'bin/neonmix-airplay-worker.exe']})
    api = None
    try:
        subprocess.run([str(hub_path), 'init', '--directory', str(base / 'profile'),
                        '--output', args.output], check=True, capture_output=True, timeout=30)
        credential = json.loads((base / 'profile/admin.json').read_text())
        tls = ssl.create_default_context(cadata=credential['certificate'])
        log_path = base / 'hub.log'
        with log_path.open('w') as log:
            hub = subprocess.Popen([str(hub_path), 'serve', '--config', str(base / 'profile/server.json'),
                                    '--listen', '127.0.0.1:0', '--managed-control-stdin'],
                                   stdin=subprocess.PIPE, stdout=log, stderr=log,
                                   env=dict(os.environ, NEONMIX_AIRPLAY_TRACE='1',
                                            NEONMIX_AIRPLAY_RUNTIME=str(args.runtime.resolve())))
        def address():
            assert hub.poll() is None, 'Hub exited during startup'
            for line in log_path.read_text().splitlines():
                try:
                    event = json.loads(line)
                except ValueError:
                    continue
                if event.get('event') == 'hub_started':
                    return event['listen']
        url = 'https://' + wait_for('Hub listening', address)
        def request(path, body=None):
            req = Request(url + path, data=None if body is None else json.dumps(body).encode(),
                          headers={'Authorization': 'Bearer ' + credential['token'],
                                   'Content-Type': 'application/json'})
            try:
                with urlopen(req, context=tls, timeout=5) as response:
                    return response.status, json.load(response)
            except HTTPError as error:
                return error.code, json.load(error)
        api = request
        def state():
            code, body = api('/v2/airplay')
            assert code == 200
            return body
        def command(action, **fields):
            for _ in range(10):
                code, body = api('/v2/airplay', dict(command_id=str(uuid.uuid4()),
                    expected_revision=state()['revision'], operation=dict(action=action, **fields)))
                if code == 409 and body.get('error') == 'stale_revision':
                    continue
                assert code == 200, (action, code, body.get('error'))
                return
            raise AssertionError('command revision retry exhausted')
        command('configure', receiver_count=args.sources, multi_receiver=True)
        for index in range(args.native_sources):
            native.append(subprocess.Popen([str(hub_path), 'send', '--credential', str(base / f'profile/sender-{chr(97 + index)}.json'), '--hub', url, '--seconds', '120'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        if native:
            wait_for('native sources render', lambda: len(api('/v1/diagnostics')[1]['media_workers']) == len(native))
        for index, receiver in enumerate(state()['receivers'][:args.sources]):
            receiver_id = receiver['receiver_id']
            def workers():
                result = powershell(f"@(Get-CimInstance Win32_Process | Where-Object {{$_.ParentProcessId -eq {hub.pid} -and $_.Name -eq 'neonmix-airplay-worker.exe'}} | Select-Object -ExpandProperty ProcessId) | ConvertTo-Json -Compress")
                values = json.loads(result) if result.strip() else []
                return set(values if isinstance(values, list) else [values])
            before = workers()
            command('enable_receiver', receiver_id=receiver_id)
            def ready():
                r = next(r for r in state()['receivers'] if r['receiver_id'] == receiver_id)
                return r if r['ready'] else None
            receiver = wait_for('receiver ready', ready, seconds=30)
            pid, = workers() - before
            port = int(powershell(f"Get-NetTCPConnection -State Listen -OwningProcess {pid} | Select-Object -First 1 -ExpandProperty LocalPort").strip())
            source = DigitalSource(index, args.crypto.resolve(), args.codec, args.omit_spf)
            sources.append(source)
            try:
                source.start(port, receiver['pairing_pin'])
            finally:
                report['stages'].append(dict(source=index + 1, stage=source.protocol_stage,
                                             response=source.last_response))
            wait_for('source releases PCM', lambda: len(state()['sessions']) == index + 1
                     and all(r['released_blocks'] > 10 for r in state()['receivers'][:index + 1]))
        before = state()
        time.sleep(5)
        after = state()
        assert len(after['sessions']) == args.sources
        assert all(b['released_blocks'] > a['released_blocks'] for a, b in zip(before['receivers'], after['receivers']))
        assert all(p.poll() is None for p in native), 'native Sender exited'
        assert after['capacity']['active'] == args.sources, 'native Sender consumed AirPlay quota'
        report['capacity'] = after['capacity']
        report['released_blocks'] = [r['released_blocks'] for r in after['receivers']]
        diagnostics = api('/v1/diagnostics')[1]
        active_lanes = [i for i, stream in enumerate(diagnostics['lane_stream_ids']) if stream is not None]
        assert len(active_lanes) == args.sources + args.native_sources
        rendered = [diagnostics['rendered_pcm_frames_by_lane'][i] for i in active_lanes]
        assert all(frames > 4800 for frames in rendered), 'an admitted lane never rendered PCM'
        report['rendered_frames'] = rendered
        report['passed'] = True
    except Exception as error:
        report['error'] = str(error)
        if (base / 'hub.log').exists():
            report['hub_exit'] = hub.poll()
            report['startup_log'] = (base / 'hub.log').read_text()[-1500:] if not report['stages'] else None
    finally:
        if api:
            try:
                snapshot = api('/v2/airplay')[1]
                report['receivers'] = [{k: r.get(k) for k in ['ready', 'active', 'error', 'failure_stage',
                    'received_blocks', 'released_blocks', 'admission_denials', 'format', 'protocol_events']}
                    for r in snapshot.get('receivers', [])]
            except Exception:
                pass
        for source in sources:
            source.close()
        for process in native:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=5)
        if hub and hub.poll() is None:
            hub.stdin.close()
            try:
                hub.wait(timeout=15)
            except subprocess.TimeoutExpired:
                hub.kill()
                hub.wait()
        report['hub_exit_code'] = hub.returncode if hub else None
        report['runtime_keys_remaining'] = len(list(base.rglob('runtime-key-*')))
        report_path.write_text(json.dumps(report, indent=2) + '\n')
        # Lab credentials and protocol logs are never retained in the report.
        shutil.rmtree(base)
        for handle in handles:
            handle.close()
    print(json.dumps({k:v for k,v in report.items() if k != 'receivers'}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
