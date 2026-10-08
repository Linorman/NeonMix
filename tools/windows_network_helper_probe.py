#!/usr/bin/env python3
"""Test only the supplied project-local helper fixtures and their exact rules.

Does not change firewall enablement, profiles, or third-party rules. An elevated
isolated Windows test session is required. This is rule-maintenance evidence,
not Public-network or Apple-device acceptance.
"""
import argparse
import base64
import configparser
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def ps(source):
    encoded = base64.b64encode(source.encode('utf-16le')).decode()
    result = subprocess.run(['powershell', '-NoProfile', '-NonInteractive', '-EncodedCommand', encoded],
                            capture_output=True, text=True, encoding='utf-8', timeout=30)
    assert result.returncode == 0, 'PowerShell fixture operation failed'
    return result.stdout.strip()


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def helper(path, operation, *args):
    result = subprocess.run([str(path), operation, *map(str, args)], capture_output=True,
                            text=True, encoding='utf-8', timeout=30)
    lines = result.stdout.splitlines()
    value = json.loads(lines[-1]) if lines else {}
    return result.returncode, value


def instance(root):
    record = configparser.ConfigParser()
    record.read(root / 'network-instance.ini', encoding='utf-16')
    return 'NeonMix.' + record['NeonMix']['Instance']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--helper', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args()
    assert os.name == 'nt'
    executable = args.helper.resolve()
    executable.relative_to(ROOT)
    destination = args.report.resolve()
    destination.relative_to(ROOT)
    root = executable.parent.parent
    assert not (root / 'network-instance.ini').exists(), 'use a fresh disposable fixture'
    checks = {}
    with tempfile.TemporaryDirectory(prefix='network-instance-', dir=ROOT / '.local/tmp') as temporary:
        second = Path(temporary) / '第二实例 é 𝄞'
        shutil.copytree(root, second)
        other = second / 'bin/neonmix-network-helper.exe'
        roots = [(root, executable), (second, other)]
        block = None
        try:
            for path, tool in roots:
                code, result = helper(tool, 'install')
                assert code == 0 and result['configuration_code'] == 0 and all(result['rules_match']), result
            groups = [instance(path) for path, _ in roots]
            assert groups[0] != groups[1]
            group = groups[0]
            script = "$ErrorActionPreference='Stop'; $p=New-Object -ComObject HNetCfg.FwPolicy2; "
            count = lambda g: int(ps(script + "@($p.Rules | Where-Object {$_.Grouping -eq " + literal(g) + "}).Count"))
            assert [count(g) for g in groups] == [4, 4]
            for _ in range(3):
                assert helper(executable, 'repair')[0] == 0
            assert count(group) == 4 and instance(root) == group
            checks['idempotent_install_and_repair'] = True
            code, result = helper(executable, 'repair', '--port', 17443)
            assert code == 0 and all(result['rules_match'])
            assert helper(executable, 'inspect')[1]['configuration_code'] == 0
            assert helper(executable, 'repair', '--port', 7443)[0] == 0
            assert helper(executable, 'repair', '--port', 65536)[0] == 20
            checks['validated_custom_control_port'] = True
            name = group + '.AirPlay-Media-UDP'
            ps(script + "$p.Rules.Item(" + literal(name) + ").ApplicationName=" + literal(root / 'bin/neonmix-hub.exe'))
            assert helper(executable, 'inspect')[1]['configuration_code'] == 2
            assert helper(executable, 'repair')[0] == 0
            assert helper(executable, 'inspect')[1]['configuration_code'] == 0
            checks['path_mismatch_detected_and_repaired'] = True
            block = group + '.Probe.Block'
            ps(script + "$r=New-Object -ComObject HNetCfg.FWRule; $r.Name=" + literal(block) +
               ";$r.ApplicationName=" + literal(root / 'airplay/bin/neonmix-airplay-worker.exe') +
               ";$r.Protocol=17;$r.Direction=1;$r.Action=0;$r.Profiles=7;$r.Enabled=$true;$p.Rules.Add($r)")
            _, observed = helper(executable, 'inspect')
            assert observed.get('matching_block_rule') is True
            helper(executable, 'repair')
            assert ps(script + "$p.Rules.Item(" + literal(block) + ").Action") == '0'
            checks['external_block_retained'] = True
            ps(script + "$p.Rules.Remove(" + literal(block) + ")")
            block = None
            assert helper(executable, 'remove')[0] == 0
            assert count(groups[0]) == 0 and count(groups[1]) == 4
            assert helper(executable, 'remove')[0] == 0
            checks['isolated_instance_and_repeated_remove'] = True
            assert helper(executable, 'inspect')[1]['configuration_code'] == 1
            checks['missing_rules_reported'] = True
            report = {'scope': 'native rule maintenance only; firewall and network profiles unchanged',
                      'checks': checks, 'profiles': observed['firewall_enabled'],
                      'effective_policy_code': observed['effective_policy_code'],
                      'helper_sha256': hashlib.sha256(executable.read_bytes()).hexdigest(),
                      'uac_cross_user': 'not_run', 'public_iphone_playback': 'not_run'}
        finally:
            if block:
                ps("$p=New-Object -ComObject HNetCfg.FwPolicy2;$p.Rules.Remove(" + literal(block) + ")")
            for _, tool in roots:
                # Only helper-owned instance names are removed. Keep files on
                # failure so the original record remains available for cleanup.
                code, _ = helper(tool, 'remove')
                assert code == 0, 'fixture rules require cleanup'
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
