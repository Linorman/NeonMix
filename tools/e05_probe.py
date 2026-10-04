#!/usr/bin/env python3
"""macOS E05: actual DNS-SD, file credentials, pinned HTTPS/WSS and protected media.

Run through tools/dev. No system routing or audio settings are changed.
All invitations and credential profiles are temporary and removed at the end.
"""
import argparse
import hashlib
import http.client
import json
import re
from pathlib import Path
import secrets
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import traceback
import uuid

from credential_fixture import remove_owned_fixture

ROOT = Path(__file__).resolve().parents[1]
if sys.platform != 'darwin':
    raise SystemExit('E05 runtime probe is macOS only')
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', default='coreaudio:BlackHole2ch_UID')
parser.add_argument('--link-local-interface', help='Also verify scoped IPv6 media on a real macOS interface')
parser.add_argument('--skip-default-listen', action='store_true', help='Skip only the product-default 7443 listener check when another project session owns that port')
parser.add_argument('--report-dir', type=Path, help='New project-local directory for this invocation logs and result')
args = parser.parse_args()
BINARY = ROOT / 'target/release/neonmix-hub'
OUT = (args.report_dir or ROOT / 'artifacts/e05' / time.strftime('%Y%m%d-%H%M%S')).resolve()
OUT.relative_to(ROOT)
OUT.mkdir(parents=True)
LAB = Path(tempfile.mkdtemp(prefix='e05-', dir=ROOT / '.local/tmp'))
children, handles = [], []
report = {'passed': False, 'platform': sys.platform, 'scope': 'macOS local functional E05; no Windows/Linux runtime or cross-machine performance claim',
          'binary_sha256': hashlib.sha256(BINARY.read_bytes()).hexdigest(), 'scenarios': {}}

def run(*arguments, ok=True, timeout=25):
    result = subprocess.run([str(BINARY), *map(str, arguments)], cwd=ROOT, capture_output=True, text=True, timeout=timeout)
    assert (result.returncode == 0) == ok, result.stderr
    return [json.loads(line) for line in result.stdout.splitlines()] if ok else result.stderr.strip()

def start(name, *arguments):
    stdout, stderr = (OUT / f'{name}.jsonl').open('w'), (OUT / f'{name}.stderr').open('w')
    handles.extend([stdout, stderr])
    child = subprocess.Popen([str(BINARY), *map(str, arguments)], cwd=ROOT, stdout=stdout, stderr=stderr)
    children.append(child)
    return child

def rows(name):
    return [json.loads(line) for line in (OUT / f'{name}.jsonl').read_text().splitlines()]

def stop(child):
    if child.poll() is None:
        child.send_signal(signal.SIGINT)
        try:
            child.wait(timeout=8)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()

def wait(getter, predicate=bool, seconds=12):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        value = getter()
        if predicate(value):
            return value
        time.sleep(.1)
    raise TimeoutError('expected E05 state did not arrive')

def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]

def hub(directory, name, listen):
    child = start(name, 'serve', '--config', directory / 'server.json', '--listen', listen)
    value = wait(lambda: rows(name), seconds=12)[0]
    assert value['event'] == 'hub_started'
    return child, value

def invite(directory, url, name, seconds=120):
    path = LAB / f'{name}.invite.json'
    run('invite', '--credential', directory / 'admin.json', '--hub', url, '--out', path, '--seconds', seconds)
    return path, json.loads(path.read_text())

def raw(url, certificate, path, token='', body=None):
    address = url.removeprefix('https://')
    host, raw_port = address.rsplit(':', 1)
    context = ssl.create_default_context(cadata=certificate)
    context.check_hostname = False
    connection = http.client.HTTPSConnection(host.strip('[]'), int(raw_port), context=context, timeout=4)
    try:
        connection.connect()
        expected = hashlib.sha256(ssl.PEM_cert_to_DER_cert(certificate)).digest()
        assert hashlib.sha256(connection.sock.getpeercert(binary_form=True)).digest() == expected
        headers = {'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'}
        connection.request('POST' if body is not None else 'GET', path, None if body is None else json.dumps(body), headers)
        response = connection.getresponse()
        return response.status, json.loads(response.read())
    finally:
        connection.close()

def snapshot(directory, url=None):
    arguments = ['snapshot', '--credential', directory / 'admin.json']
    if url:
        arguments += ['--hub', url]
    return run(*arguments)[0]

def command(directory, url, operation, name):
    state = snapshot(directory, url)
    path = LAB / f'{name}.command.json'
    path.write_text(json.dumps({'request_id': str(uuid.uuid4()), 'expected_revision': state['revision'], 'operation': operation}))
    return run('control', '--credential', directory / 'admin.json', '--hub', url, '--command', path)[0]

try:
    run('credential-store-probe', '--directory', LAB)
    report['scenarios']['file_store_write_read_delete'] = True
    real, fake = LAB / 'real', LAB / 'fake'
    configured = run('setup', '--directory', real, '--output', args.output, '--name', 'E05 同名房间')[0]
    run('setup', '--directory', fake, '--output', args.output, '--name', 'E05 同名房间')
    p1, p2 = port(), port()
    url, fake_url = f'https://127.0.0.1:{p1}', f'https://127.0.0.1:{p2}'
    original, _ = hub(real, 'hub', f'0.0.0.0:{p1}')
    impostor, _ = hub(fake, 'fake', f'127.0.0.1:{p2}')
    discovered = run('discover', '--seconds', 3)
    candidates = discovered[-1]['candidates']
    matched = [c for c in candidates if c['room_name'] == 'E05 同名房间']
    assert len(matched) == 2 and len({c['hub_id'] for c in matched}) == 2
    report['scenarios']['same_name_candidates'] = matched
    invitation_path, invitation = invite(real, url, 'first')
    profile = LAB / 'sender.json'
    began=time.monotonic()
    paired = run('pair', '--invite', invitation_path, '--credential', profile, '--name', 'E05 Sender')[0]
    report['pairing_seconds_single_sample']=round(time.monotonic()-began,4)
    assert paired['hub_id'] == configured['hub_id']
    data = json.loads(profile.read_text())
    assert data['version'] == 2 and data['credential_store'] == 'file' and data['profile_kind'] == 'member'
    assert configured['credential_store'] == 'file'
    assert not data['pending'] and 'token' not in data and 'private_key' not in json.loads((real/'server.json').read_text())
    assert (profile.stat().st_mode & 0o777) == 0o600
    assert (invitation_path.stat().st_mode & 0o777) == 0o600
    run('snapshot', '--credential', profile)
    assert run('pair', '--invite', invitation_path, '--credential', profile, '--name', 'E05 Sender')[0]['device_id'] == paired['device_id']
    report['scenarios']['automatic_pair_and_repeat_file_profile'] = paired
    assert raw(url, invitation['certificate'], '/v1/hub', secrets.token_hex(32))[0] == 401
    denial = run('snapshot', '--credential', profile, '--hub', fake_url, ok=False)
    report['scenarios']['unpaired_and_wrong_certificate_rejected'] = {'http': 401, 'client_error': denial}
    # A duplicate stable UUID with a different key must not replace saved trust.
    stop(impostor)
    fake_state=json.loads((fake/'state.json').read_text());fake_state['hub_id']=configured['hub_id']
    (fake/'state.json').write_text(json.dumps(fake_state))
    fake_admin=json.loads((fake/'admin.json').read_text());fake_admin['hub_id']=configured['hub_id']
    (fake/'admin.json').write_text(json.dumps(fake_admin))
    impostor,_=hub(fake,'changed-key',f'127.0.0.1:{p2}')
    denial=run('snapshot','--credential',profile,'--hub',fake_url,ok=False)
    run('snapshot','--credential',profile)
    duplicates=run('discover','--seconds',3)[-1]['candidates']
    same_ids=[c for c in duplicates if c['hub_id']==configured['hub_id']]
    assert len(same_ids)==2 and len({c['instance'] for c in same_ids})==2
    report['scenarios']['changed_key_rejected_and_duplicate_identity_candidates']={'client_error':denial,'instances':[c['instance'] for c in same_ids]}
    stop(impostor)
    # Identical certificate/key and credential state, deliberately changed UUID.
    clone = LAB / 'clone'
    shutil.copytree(real, clone)
    state = json.loads((real/'state.json').read_text()); state['hub_id'] = str(uuid.uuid4())
    (clone/'state.json').write_text(json.dumps(state))
    clone_admin=json.loads((clone/'admin.json').read_text());clone_admin['hub_id']=state['hub_id']
    (clone/'admin.json').write_text(json.dumps(clone_admin))
    config = json.loads((real/'server.json').read_text()); config['state_path'] = 'state.json'
    (clone/'server.json').write_text(json.dumps(config))
    p3 = port(); cloned, _ = hub(clone, 'clone', f'127.0.0.1:{p3}')
    denial = run('snapshot', '--credential', profile, '--hub', f'https://127.0.0.1:{p3}', ok=False)
    assert 'UUID' in denial
    stop(cloned)
    report['scenarios']['same_certificate_foreign_hub_uuid_rejected'] = denial
    # Direct retry uses one client-generated token: grant cannot enroll a second device.
    retry_path, retry = invite(real, url, 'retry')
    token = secrets.token_hex(32)
    request = {'invitation_id': retry['invitation_id'], 'request_id': str(uuid.uuid4()), 'name': 'Raw Retry', 'token_sha256': hashlib.sha256(token.encode()).hexdigest()}
    status, receipt = raw(url, retry['certificate'], '/v1/pairing/complete', retry['secret'], request)
    assert status == 200
    assert raw(url, retry['certificate'], '/v1/pairing/complete', retry['secret'], request) == (200, receipt)
    changed = dict(request, request_id=str(uuid.uuid4()))
    assert raw(url, retry['certificate'], '/v1/pairing/complete', retry['secret'], changed)[0] == 409
    report['scenarios']['single_use_exact_retry'] = receipt
    # A real atomic-rename failure must not consume the grant or register identity.
    disk_invite_path,disk_invite=invite(real,url,'disk-failure')
    disk_token=secrets.token_hex(32)
    disk_request={'invitation_id':disk_invite['invitation_id'],'request_id':str(uuid.uuid4()),'name':'Disk Retry','token_sha256':hashlib.sha256(disk_token.encode()).hexdigest()}
    disk_state=real/'state.json';saved_state=real/'state-original.json'
    disk_bytes=disk_state.read_bytes();before_revision=snapshot(real,url)['revision']
    disk_state.rename(saved_state)
    try:
        disk_state.mkdir()
        status,error=raw(url,disk_invite['certificate'],'/v1/pairing/complete',disk_invite['secret'],disk_request)
        assert status==503 and error['error']=='busy'
        assert snapshot(real,url)['revision']==before_revision
        assert raw(url,disk_invite['certificate'],'/v1/hub',disk_token)[0]==401
    finally:
        if disk_state.is_dir():disk_state.rmdir()
        saved_state.rename(disk_state)
    assert disk_state.read_bytes()==disk_bytes
    assert not list(real.glob('.neonmix-*.tmp'))
    assert raw(url,disk_invite['certificate'],'/v1/pairing/complete',disk_invite['secret'],disk_request)[0]==200
    report['scenarios']['actual_rename_failure_is_atomic_and_grant_retryable']=True
    cancelled_path, cancelled = invite(real, url, 'cancelled')
    run('cancel-invite', '--credential', real/'admin.json', '--hub', url, '--invitation-id', cancelled['invitation_id'])
    canceled_profile = LAB/'cancelled-sender.json'
    cancelled_error = run('pair', '--invite', cancelled_path, '--credential', canceled_profile, '--name', 'Cancelled', '--hub', url, ok=False)
    assert 'pairing rejected:' in cancelled_error and '401' in cancelled_error
    expired_path, expired = invite(real, url, 'expired', seconds=1)
    time.sleep(1.2)
    assert 'expired' in run('pair', '--invite', expired_path, '--credential', LAB/'expired-sender.json', '--name', 'Expired', '--hub', url, ok=False)
    bad = dict(request, invitation_id=expired['invitation_id'], request_id=str(uuid.uuid4()))
    assert raw(url, expired['certificate'], '/v1/pairing/complete', expired['secret'], bad)[0] == 401
    report['scenarios']['cancel_and_client_server_expiry'] = True
    # Begin actual protected media from the file credential.
    sending = start('sender', 'send', '--credential', profile, '--seconds', 30, '--frequency', 437)
    watch = start('watch', 'watch', '--credential', profile, '--seconds', 30)
    def diagnostics():
        return run('diagnostics', '--credential', real/'admin.json', '--hub', url)[0]
    before = wait(diagnostics, lambda d: d['receivers'] and d['receivers'][0]['pcm_frames'] > 48000)
    assert before['receivers'][0]['authenticated']
    command(real, url, {'type': 'revoke', 'device_id': paired['device_id']}, 'revoke')
    assert sending.wait(timeout=8) != 0
    assert watch.wait(timeout=8) != 0
    assert not any(s['status'] in ('buffering','playing','network_degraded') for s in snapshot(real,url)['sessions'].values())
    run('snapshot', '--credential', profile, '--hub', url, ok=False)
    report['scenarios']['revoke_control_and_active_protected_media'] = {'pcm_before': before['receivers'][0]['pcm_frames'], 'sender_exit': sending.returncode, 'watch_exit': watch.returncode}
    # Observe goodbye removal and then rediscover the persisted Hub at another port.
    browser = start('browser', 'discover', '--seconds', 18)
    wait(lambda: rows('browser'), lambda values: any(v.get('candidate',{}).get('hub_id')==configured['hub_id'] for v in values))
    restart_path, restart_invite = invite(real,url,'restart')
    stop(original)
    wait(lambda: rows('browser'), lambda values: any(v['event']=='removed' and v['candidate']['hub_id']==configured['hub_id'] for v in values))
    new_port = port(); new_url = f'https://127.0.0.1:{new_port}'
    restarted, _ = hub(real,'restarted',f'127.0.0.1:{new_port}')
    assert snapshot(real)['hub_id'] == configured['hub_id']
    assert raw(new_url,retry['certificate'],'/v1/hub',token)[0] == 200
    request['invitation_id']=restart_invite['invitation_id'];request['request_id']=str(uuid.uuid4());request['token_sha256']=hashlib.sha256(secrets.token_bytes(32)).hexdigest()
    assert raw(new_url,restart_invite['certificate'],'/v1/pairing/complete',restart_invite['secret'],request)[0] == 401
    run('snapshot','--credential',profile,ok=False)
    report['scenarios']['goodbye_address_change_persistent_trust_revocation_and_grant_expiry'] = {'old_port': p1,'new_port':new_port,'stable_hub_id':configured['hub_id']}
    # A restored pending local profile recovers a durable registration without creating another device.
    recovery_invite_path, _ = invite(real,new_url,'recovery')
    recovered_profile=LAB/'recovered.json'
    recovered=run('pair','--invite',recovery_invite_path,'--credential',recovered_profile,'--name','Recovery','--hub',new_url)[0]
    meta=json.loads(recovered_profile.read_text());meta['pending']=True;recovered_profile.write_text(json.dumps(meta))
    assert run('pair','--invite',recovery_invite_path,'--credential',recovered_profile,'--name','Recovery','--hub',new_url)[0]['device_id']==recovered['device_id']
    report['scenarios']['lost_response_local_recovery'] = True
    # Credential store loss fails closed, without a laboratory-token fallback.
    meta=json.loads(recovered_profile.read_text());meta['secret_ref']=str(uuid.uuid4())
    missing=LAB/'missing.json';missing.write_text(json.dumps(meta));missing.chmod(0o600)
    run('snapshot','--credential',missing,'--hub',new_url,ok=False)
    report['scenarios']['missing_file_secret_fails_closed'] = True
    browser.wait(timeout=20)
    # Control subscription re-discovers the same identity at an IPv6 endpoint.
    live_watch=start('moving-watch','watch','--credential',recovered_profile,'--seconds',30)
    wait(lambda:rows('moving-watch'),lambda values:any(v.get('connected') for v in values))
    stop(restarted)
    ipv6_port=port();ipv6_url=f'https://[::1]:{ipv6_port}'
    ipv6_hub,_=hub(real,'ipv6','[::1]:'+str(ipv6_port))
    wait(lambda:rows('moving-watch'),lambda values:any(v.get('connected') and v.get('subscriptions',0)>=2 for v in values),seconds=16)
    run('snapshot','--credential',recovered_profile)
    ipv6_sender=start('ipv6-sender','send','--credential',recovered_profile,'--seconds',10)
    def ipv6_diagnostics():
        return run('diagnostics','--credential',real/'admin.json','--hub',ipv6_url)[0]
    ipv6_stats=wait(ipv6_diagnostics,lambda d:d['receivers'] and d['receivers'][0]['authenticated'] and d['receivers'][0]['pcm_frames']>48000)
    assert ipv6_sender.wait(timeout=12)==0
    report['scenarios']['ipv6_discovery_media_and_wss_address_recovery']={'media_frames':ipv6_stats['receivers'][0]['pcm_frames'],'subscriptions':max(v.get('subscriptions',0) for v in rows('moving-watch'))}
    stop(live_watch)
    if args.link_local_interface:
        stop(ipv6_hub)
        interface_text=subprocess.check_output(['ifconfig',args.link_local_interface],text=True)
        match=re.search(r'inet6 (fe80:[^%\s]+)%',interface_text)
        if not match:raise RuntimeError('selected interface has no IPv6 link-local address')
        scope=socket.if_nametoindex(args.link_local_interface)
        scoped_port=port();scoped_listen=f'[{match.group(1)}%{scope}]:{scoped_port}'
        ipv6_hub,_=hub(real,'link-local',scoped_listen)
        selected=run('discover','--seconds',3)[-1]['candidates']
        scoped_candidates=[candidate for candidate in selected if candidate['hub_id']==configured['hub_id']]
        assert scoped_candidates and any(f'%{scope}]' in endpoint['address'] for candidate in scoped_candidates for endpoint in candidate['endpoints'])
        scoped_sender=start('link-local-sender','send','--credential',recovered_profile,'--seconds',10)
        def scoped_diagnostics():
            return run('diagnostics','--credential',real/'admin.json')[0]
        scoped_stats=wait(scoped_diagnostics,lambda d:d['receivers'] and d['receivers'][0]['authenticated'] and d['receivers'][0]['pcm_frames']>48000)
        assert scoped_sender.wait(timeout=12)==0
        report['scenarios']['real_link_local_scope_https_wss_and_protected_media']={'interface':args.link_local_interface,'scope':scope,'media_frames':scoped_stats['receivers'][0]['pcm_frames']}
    # Abrupt loss has no goodbye; the browser's active verification removes it.
    abrupt_browser=start('abrupt-browser','discover','--seconds',10)
    wait(lambda:rows('abrupt-browser'),lambda values:any(v.get('candidate',{}).get('hub_id')==configured['hub_id'] for v in values))
    ipv6_hub.kill();ipv6_hub.wait(timeout=8)
    wait(lambda:rows('abrupt-browser'),lambda values:any(v['event']=='removed' and v['candidate']['hub_id']==configured['hub_id'] for v in values),seconds=8)
    abrupt_browser.wait(timeout=12)
    report['scenarios']['abrupt_offline_active_verification']=True
    if args.skip_default_listen:
        report['not_run']={'product_default_listen_is_discoverable_lan': 'Explicit --skip-default-listen; other session may own 7443'}
    else:
        # The ordinary product entry publishes to LAN without a diagnostic --listen.
        with socket.socket() as reserve:
            reserve.bind(('0.0.0.0',7443))
        default_hub=start('default-listen','serve','--config',real/'server.json')
        started=wait(lambda:rows('default-listen'))[0]
        assert started['listen']=='0.0.0.0:7443'
        assert snapshot(real)['hub_id']==configured['hub_id']
        report['scenarios']['product_default_listen_is_discoverable_lan']=True
        stop(default_hub)
    report['passed'] = True
except Exception as error:
    report['error'] = str(error)
    report['traceback'] = traceback.format_exc()
finally:
    for child in reversed(children):
        stop(child)
    for handle in handles:
        handle.close()
    remove_owned_fixture(LAB)
    report['cleanup']={'credential_store':'file','temporary_fixture_removed':not LAB.exists()}
    (OUT/'result.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps({'passed':report['passed'],'report':str(OUT/'result.json'),'error':report.get('error')},ensure_ascii=False))
raise SystemExit(0 if report['passed'] else 1)
