#!/usr/bin/env python3
"""Regress the persisted Ed25519 identity path, including full worker startup.

Run with tools/dev (or tools/dev.ps1). Fixtures use the public RFC 8032 test
vector; no real user identity is read. All temporary files stay in the project.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import queue
import socket
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
SEED = bytes.fromhex('9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60')
PUBLIC = 'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a'
SIGNATURE = ('e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555f'
             'b8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b')


def pem(der, label='PRIVATE KEY'):
    encoded = base64.b64encode(der).decode()
    lines = [encoded[i:i+64] for i in range(0, len(encoded), 64)]
    return (f'-----BEGIN {label}-----\n' + '\n'.join(lines) +
            f'\n-----END {label}-----\n').encode()


def fixtures():
    v1 = bytes.fromhex('302e020100300506032b657004220420') + SEED
    v2 = bytes.fromhex('3051020101300506032b657004220420') + SEED + bytes.fromhex('812100' + PUBLIC)
    legacy = bytes.fromhex('3053020101300506032b657004220420') + SEED + bytes.fromhex('a123032100' + PUBLIC)
    values = dict(v1=(pem(v1), True), ring_v2=(pem(v2), True),
                  legacy_ring_v2=(pem(legacy), True),
                  crlf_v2=(pem(v2).replace(b'\n', b'\r\n'), True),
                  missing=(None, False), empty=(b'', False),
                  truncated_pem=(pem(v2)[:-25], False),
                  truncated_der=(pem(v2[:-1]), False),
                  wrong_algorithm=(pem(v1[:11] + b'\x6e' + v1[12:]), False),
                  wrong_v2_public=(pem(v2[:-1] + bytes([v2[-1] ^ 1])), False),
                  wrong_v2_seed=(pem(v2[:16] + bytes([v2[16] ^ 1]) + v2[17:]), False),
                  wrong_version=(pem(v2[:4] + b'\x00' + v2[5:]), False),
                  wrong_public_tag=(pem(v2[:48] + b'\x82' + v2[49:]), False),
                  trailing_der=(pem(v2 + b'\x00'), False),
                  trailing_pem=(pem(v1) + pem(v2), False),
                  encrypted_label=(pem(v1, 'ENCRYPTED PRIVATE KEY'), False),
                  boundary_4095=(pem(v1) + b' ' * (4095-len(pem(v1))), True),
                  boundary_4096=(pem(v1) + b' ' * (4096-len(pem(v1))), False),
                  trailing_garbage=(pem(v1) + b'x', False),
                  oversized=(b'x' * 4097, False))
    values['中文 é 𝄞 with spaces'] = (pem(v1), True)
    return values


def worker_start(worker, path, valid, directory, plugins, stop_mode="command"):
    with socket.socket() as media:
        media.bind(('127.0.0.1', 0))
        media.listen(1)
        media.settimeout(5)
        env = dict(os.environ, GST_REGISTRY=str(directory / 'registry.bin'),
                   GST_PLUGIN_SYSTEM_PATH_1_0=str(plugins))
        process = subprocess.Popen([str(worker)], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, env=env)
        events = queue.Queue()
        def reader():
            for line in process.stdout:
                events.put(json.loads(line))
            events.put(None)
        thread = threading.Thread(target=reader, daemon=True)
        thread.start()
        connection = None
        config = dict(control_version=2, pcm_version=2, session_control_version=1, worker_generation=7, trust_generation=1,
                      pairing_remaining_ms=600000, pairing_attempts=0,
                      media_address=f'127.0.0.1:{media.getsockname()[1]}', ipc_token='a'*64,
                      device_id='001122334455', receiver_uuid='00112233-4455-6677-8899-aabbccddeeff',
                      keyfile=str(path), name='NeonMix identity probe', pin='1234', rtsp_port=0,
                      session_id=1, stream_id=2, stream_epoch=1, format_epoch=1, mapping_id=1,
                      pairing_allowed=True, known_client_keys=[], blocked_client_keys=[])
        try:
            process.stdin.write(json.dumps(config)+'\n')
            process.stdin.flush()
            media.settimeout(.05)
            deadline=time.monotonic()+5
            while time.monotonic()<deadline:
                try:
                    connection, _ = media.accept()
                    connection.settimeout(5)
                    break
                except socket.timeout:
                    if process.poll() is not None:break
            if not connection:
                if valid:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)
                    raise AssertionError(f'valid identity failed to establish IPC: {list(events.queue)}, {process.stderr.read()}')
            ready = False
            failures = []
            while True:
                event = events.get(timeout=10)
                if event is None:
                    break
                if event.get('type') == 'fatal':
                    failures.append(event.get('message', ''))
                if event.get('type') == 'ready':
                    assert valid, 'invalid identity reached ready'
                    assert event.get('identity_loader_version')==1 and event.get('stop_version')==1, 'reliability capability missing'
                    assert event['public_key'] == PUBLIC, 'identity public key changed'
                    ready = True
                    if stop_mode=="eof":process.stdin.close()
                    else:
                        process.stdin.write('{"type":"stop"}\n')
                        process.stdin.flush()
                    break
            code = process.wait(timeout=10)
            stderr = process.stderr.read()
            assert code == (0 if valid else 1), f'unexpected worker exit {code}: {stderr}'
            assert ready == valid, f'wrong readiness: {stderr}'
            assert 'OPENSSL_Applink' not in stderr
            return dict(exit_code=code, ready=ready, errors=failures)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            if connection:
                connection.close()
            thread.join(2)
            process.stdin.close()
            process.stdout.close()
            process.stderr.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe', type=Path, default=ROOT / '.local/airplay/build/neonmix-airplay-identity-probe')
    parser.add_argument('--worker', type=Path, help='Also test actual worker ready/error behavior')
    parser.add_argument('--report', type=Path)
    parser.add_argument('--plugins', type=Path, default=ROOT/'.local/airplay/plugins')
    parser.add_argument('--dll-dir', action='append', default=[])
    args = parser.parse_args()
    os.environ["PATH"]=os.pathsep.join(args.dll_dir+[os.environ.get("PATH","")])
    os.environ["GST_REGISTRY_FORK"]="no"
    assert Path(os.environ.get('TMPDIR', os.environ.get('TEMP', '/'))).resolve().is_relative_to(ROOT), 'run through tools/dev'
    checks = {}
    startup = {}
    with tempfile.TemporaryDirectory(prefix='airplay-identity-', dir=ROOT / '.local/tmp') as temporary:
        directory = Path(temporary)
        for name, (content, valid) in fixtures().items():
            path = directory / (name + '.pem')
            if content is not None:
                path.write_bytes(content)
                if os.name == 'nt':
                    from credential_store_probe import protect_fixture
                    protect_fixture(path)
                else:
                    path.chmod(0o600)
            result = subprocess.run([str(args.probe.resolve()), str(path)], capture_output=True, text=True, timeout=10)
            assert result.returncode == (0 if valid else 1), f'{name}: unexpected exit {result.returncode}: {result.stderr}'
            if valid:
                assert result.stdout.splitlines() == [PUBLIC, SIGNATURE], f'{name}: identity/signature changed'
            if args.worker:
                startup[name] = worker_start(args.worker.resolve(), path, valid, directory, args.plugins)
            assert (path.read_bytes() if path.exists() else None) == content, f'{name}: persisted identity changed'
            checks[name] = 'same identity and signature' if valid else 'clean rejection'
        # Exercise file-object validation through the actual platform reader.
        path = directory / 'permission.pem'
        path.write_bytes(pem(bytes.fromhex('302e020100300506032b657004220420') + SEED))
        if os.name == 'nt':
            import ctypes
            from credential_store_probe import protect_fixture
            protect_fixture(path)
            checks['windows_acp'] = ctypes.windll.kernel32.GetACP()
            # ADS must fail before opening, even though base file is private.
            result = subprocess.run([str(args.probe.resolve()), str(path)+':secret'], capture_output=True, timeout=10)
            assert result.returncode == 1
            checks['alternate_data_stream'] = 'clean rejection'
        else:
            path.chmod(0o644)
            result = subprocess.run([str(args.probe.resolve()), str(path)], capture_output=True, timeout=10)
            assert result.returncode == 1
            path.chmod(0o600)
            linked = directory / 'link.pem'
            linked.symlink_to(path)
            result = subprocess.run([str(args.probe.resolve()), str(linked)], capture_output=True, timeout=10)
            assert result.returncode == 1
            checks['overbroad_permissions'] = checks['symlink'] = 'clean rejection'
        result = subprocess.run([str(args.probe.resolve()), str(directory)], capture_output=True, timeout=10)
        assert result.returncode == 1
        checks['directory'] = 'clean rejection'
        original=directory/'same-handle.pem'
        replacement=directory/'replacement.pem'
        valid_pem=fixtures()['v1'][0]
        original.write_bytes(valid_pem);replacement.write_bytes(fixtures()['wrong_algorithm'][0])
        if os.name=='nt':
            protect_fixture(original);protect_fixture(replacement)
        else:original.chmod(0o600);replacement.chmod(0o600)
        result=subprocess.run([str(args.probe.resolve()),str(original),str(replacement)],capture_output=True,text=True,timeout=10)
        assert result.returncode==0 and result.stdout.splitlines()==[PUBLIC,SIGNATURE], 'reopened an unvalidated replacement'
        checks['same_validated_handle']='replacement denied' if os.name=='nt' else 'original open object retained'
        if args.worker:
            original.write_bytes(valid_pem)
            if os.name=='nt':protect_fixture(original)
            startup['owner_eof']=worker_start(args.worker.resolve(),original,True,directory,args.plugins,'eof')
            startup['embedded_nul']=worker_start(args.worker.resolve(),str(original)+'\0suffix',False,directory,args.plugins)
    report = dict(platform=os.sys.platform, checks=checks,
                  worker_startup=bool(args.worker), fixtures='RFC 8032 section 7.1 test 1',
                  probe_sha256=hashlib.sha256(args.probe.read_bytes()).hexdigest())
    if args.worker:
        report['worker_sha256'] = hashlib.sha256(args.worker.read_bytes()).hexdigest()
        report['startup'] = startup
    if args.report:
        destination = args.report.resolve()
        assert destination.is_relative_to(ROOT), 'report must remain in project'
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
