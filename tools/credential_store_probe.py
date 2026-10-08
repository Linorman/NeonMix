#!/usr/bin/env python3
"""File credentials and setup failures without GUI, audio devices or native vault.

Run via tools/dev. --identity-only requires no media SDK; the default CLI checks
use a built debug Hub, or --release, but never start it or open an audio device.
All generated secrets and failure fixtures are removed from project .local/tmp.
"""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
import zipfile

from archive_policy import assert_clean, copy_ignore, excluded
from credential_fixture import remove_owned_fixture

ROOT = Path(__file__).resolve().parents[1]


def protect_fixture(path):
    """Set current-user-only permissions on files created by this invocation."""
    if os.name != 'nt':
        path.chmod(0o700 if path.is_dir() else 0o600)
        return
    row = next(csv.reader(subprocess.check_output(
        ['whoami', '/user', '/fo', 'csv', '/nh'], text=True).strip().splitlines()))
    sid = row[-1]
    if not sid.startswith('S-1-'):
        raise RuntimeError('fixture account SID unavailable')
    access = '(OI)(CI)F' if path.is_dir() else 'F'
    result = subprocess.run(['icacls', str(path), '/setowner', '*' + sid],
                            capture_output=True)
    if result.returncode:
        raise RuntimeError('private fixture owner setup failed')
    # Python 3.13's Windows mkdir(mode=0o700) adds explicit SYSTEM and
    # Administrators ACEs, which disabling inheritance alone does not remove.
    result = subprocess.run(['icacls', str(path), '/remove:g',
                             '*S-1-5-18', '*S-1-5-32-544'],
                            capture_output=True)
    if result.returncode:
        raise RuntimeError('private fixture extra ACE cleanup failed')
    result = subprocess.run(['icacls', str(path), '/inheritance:r', '/grant:r', '*' + sid + ':' + access],
                            capture_output=True)
    if result.returncode:
        raise RuntimeError('private fixture ACL setup failed')


def private_json(path, value):
    path.write_text(json.dumps(value), encoding='utf-8')
    protect_fixture(path)


def archive_checks(fixture):
    source, dest = fixture / 'archive-source', fixture / 'archive-destination'
    source.mkdir()
    marker = 'archive-secret-' + uuid.uuid4().hex
    paths = ['.credentials/key.json', 'hub/.credentials/key.json',
             'hub/airplay/runtime-key-' + str(uuid.uuid4()),
             '.neonmix-migration-' + str(uuid.uuid4()) + '.staging/key.json',
             'staging/secret.json',
             '.local/tmp/credential-migration-fixture/key.json']
    for name in paths:
        path = source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(marker, encoding='utf-8')
    (source / 'summary.json').write_text('{"passed":true}', encoding='utf-8')
    # Mirrors collect_build's copy and zip policy, including hidden stores.
    shutil.copytree(source, dest, ignore=copy_ignore)
    assert_clean(dest)
    with zipfile.ZipFile(fixture / 'archive.zip', 'w') as archive:
        for path in dest.rglob('*'):
            if path.is_file() and not excluded(path.relative_to(dest)):
                archive.write(path, path.relative_to(dest))
    with zipfile.ZipFile(fixture / 'archive.zip') as archive:
        assert archive.namelist() == ['summary.json'], 'secret path archived'
        assert all(marker.encode() not in archive.read(name) for name in archive.namelist()), 'secret bytes archived'
    # A previously populated output directory must be refused, not republished.
    leaked = dest / '.credentials'
    leaked.mkdir()
    try:
        assert_clean(dest)
    except RuntimeError:
        pass
    else:
        raise AssertionError('stale secret output was accepted')


def cli_checks(binary, fixture, scenarios):
    secrets = []

    def run(*args, code=None, cwd=None):
        result = subprocess.run([str(binary), *map(str, args)], cwd=cwd or ROOT,
                                capture_output=True, text=True, timeout=15)
        output = result.stdout + result.stderr
        assert all(value not in output for value in secrets), 'CLI leaked fixture secret'
        if code is None:
            assert result.returncode == 0, 'CLI command failed: ' + str(args[0])
            return [json.loads(line) for line in result.stdout.splitlines()]
        assert result.returncode != 0 and code in output, 'wrong failure: ' + code
        return output

    probe_directory = fixture / 'round-trip'
    probe_directory.mkdir(mode=0o700)
    protect_fixture(probe_directory)
    sentinel = probe_directory / 'keep.txt'
    sentinel.write_text('fixture owner', encoding='utf-8')
    before = set(probe_directory.iterdir())
    verified = run('credential-store-probe', '--directory', probe_directory)
    assert any(event.get('credential_store') == 'file' for event in verified)
    assert set(probe_directory.iterdir()) == before and sentinel.read_text() == 'fixture owner'
    scenarios['file_round_trip_probe_cleans_only_owned_child'] = True
    run('vault-probe', code='vault-probe')
    scenarios['obsolete_native_probe_rejected'] = True

    state = fixture / 'hub'
    run('setup', '--directory', state, '--output', 'credential-store-probe:unused', '--name', 'File probe room')
    server = json.loads((state / 'server.json').read_text())
    admin = json.loads((state / 'admin.json').read_text())
    assert server['version'] == admin['version'] == 2
    assert server['credential_store'] == admin['credential_store'] == 'file'
    assert admin['profile_kind'] == 'admin' and admin['secret_ref'] == server['admin_token_ref']
    assert server['state_path'] == 'state.json'
    store = state / '.credentials'
    for reference, kind in [(server['private_key_ref'], 'hub_tls_key'), (admin['secret_ref'], 'admin_token')]:
        entry = store / (reference + '.json')
        data = json.loads(entry.read_text())
        assert data['version'] == 1 and data['kind'] == kind and data['value']
        secrets.append(data['value'])
        if os.name != 'nt':
            assert entry.stat().st_mode & 0o777 == 0o600
    if os.name != 'nt':
        assert store.stat().st_mode & 0o777 == 0o700
        assert (state / 'server.json').stat().st_mode & 0o777 == 0o600
    snapshot = {str(path.relative_to(state)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in state.rglob('*') if path.is_file()}
    run('setup', '--directory', state, '--output', 'credential-store-probe:unused', '--name', 'File probe room', code='setup_incomplete')
    assert snapshot == {str(path.relative_to(state)): hashlib.sha256(path.read_bytes()).hexdigest()
                        for path in state.rglob('*') if path.is_file()}, 'setup changed existing identity'
    scenarios['setup_commits_shared_admin_reference_and_preserves_identity'] = True

    incomplete = fixture / 'incomplete'
    incomplete.mkdir(mode=0o700)
    protect_fixture(incomplete)
    (incomplete / 'unrelated.txt').write_text('do not overwrite', encoding='utf-8')
    run('setup', '--directory', incomplete, '--output', 'credential-store-probe:unused', '--name', 'File probe room', code='setup_incomplete')
    assert (incomplete / 'unrelated.txt').read_text() == 'do not overwrite'
    assert not (incomplete / 'server.json').exists()
    scenarios['incomplete_setup_not_overwritten'] = True

    state_path = state / 'state.json'
    saved_state = state_path.read_bytes()
    try:
        state_path.unlink()
        run('serve', '--config', state / 'server.json', code='setup_incomplete')
        assert not state_path.exists(), 'startup recreated missing authority'
        state_path.write_text('{', encoding='utf-8')
        protect_fixture(state_path)
        run('serve', '--config', state / 'server.json', code='credential_corrupt')
        assert state_path.read_text() == '{', 'startup replaced corrupted authority'
    finally:
        state_path.write_bytes(saved_state)
        protect_fixture(state_path)
    scenarios['formal_hub_missing_or_corrupt_authority_never_reinitialized'] = True

    # Load failures occur before discovery/network, from a different cwd.
    profile = fixture / 'missing.json'
    private_json(profile, dict(admin, secret_ref=str(uuid.uuid4())))
    run('snapshot', '--credential', profile, code='credential_missing', cwd=fixture)
    assert not (fixture / '.credentials').exists(), 'read created missing store'
    scenarios['profile_directory_isolation_and_missing_entry_fail_closed'] = True
    token_entry = store / (admin['secret_ref'] + '.json')
    original = token_entry.read_bytes()
    data = json.loads(original)
    try:
        private_json(token_entry, dict(data, kind='member_token'))
        run('snapshot', '--credential', state / 'admin.json', code='credential_kind_mismatch')
        private_json(token_entry, dict(data, version=999))
        run('snapshot', '--credential', state / 'admin.json', code='credential_version_unsupported')
        token_entry.write_text('{', encoding='utf-8')
        run('snapshot', '--credential', state / 'admin.json', code='credential_corrupt')
        token_entry.unlink()
        run('snapshot', '--credential', state / 'admin.json', code='credential_missing')
        # Metadata alone protects admin deletion even if its secret is missing.
        result = subprocess.run([str(binary), 'forget', '--credential', str(state / 'admin.json')],
                                cwd=ROOT, capture_output=True, text=True, timeout=15)
        assert result.returncode != 0 and (state / 'admin.json').exists(), 'admin deletion was accepted'
        assert all(value not in result.stdout + result.stderr for value in secrets), 'forget leaked secret'
    finally:
        token_entry.write_bytes(original)
        protect_fixture(token_entry)
    scenarios['corrupt_unknown_version_wrong_kind_and_admin_protection'] = True
    legacy = fixture / 'legacy.json'
    old = dict(admin, version=1)
    del old['credential_store']
    del old['profile_kind']
    private_json(legacy, old)
    run('snapshot', '--credential', legacy, code='migration_required')
    scenarios['legacy_profile_requires_explicit_migration'] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--release', action='store_true')
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--identity-only', action='store_true', help='Run pure file tests and archive checks without a Hub/media SDK')
    args = parser.parse_args()
    temporary = ROOT / '.local/tmp'
    temporary.mkdir(parents=True, exist_ok=True)
    fixture = Path(tempfile.mkdtemp(prefix='credential-store-', dir=temporary))
    report = {'passed': False, 'platform': sys.platform, 'credential_store': 'file',
              'scope': 'file persistence and setup; no GUI, audio or native credential access', 'scenarios': {}}
    try:
        protect_fixture(fixture)
        archive_checks(fixture)
        report['scenarios']['archives_exclude_hidden_secrets_runtime_keys_and_staging'] = True
        if args.identity_only:
            result = subprocess.run(['cargo', 'test', '--locked', '-p', 'neonmix-identity'], cwd=ROOT)
            assert result.returncode == 0, 'identity file tests failed'
            report['scenarios']['identity_file_tests'] = True
            report['cli_checks'] = 'not run; use a built Hub for setup checks'
        else:
            binary = args.binary or ROOT / 'target' / ('release' if args.release else 'debug') / ('neonmix-hub.exe' if os.name == 'nt' else 'neonmix-hub')
            cli_checks(binary.resolve(), fixture, report['scenarios'])
        report['passed'] = True
    except (OSError, AssertionError, ValueError, subprocess.SubprocessError) as error:
        # Failure messages are local fixed labels; never dump profile/store data.
        report['error'] = str(error)
    finally:
        remove_owned_fixture(fixture)
        report['fixture_removed'] = not fixture.exists()
    output = ROOT / 'artifacts/credential-storage' / (time.strftime('%Y%m%d-%H%M%S') + '-' + uuid.uuid4().hex[:8])
    output.mkdir(parents=True)
    (output / 'result.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(dict(report, report=str(output / 'result.json')), ensure_ascii=False))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
