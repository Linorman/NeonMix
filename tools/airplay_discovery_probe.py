#!/usr/bin/env python3
"""macOS: verify the manual receiver's actual mDNS addresses and RTSP port.

No IP addresses, PINs, credentials or protocol bodies are saved. TCP requests
originate on the Mac; this does not prove a remote Apple source can connect.
"""
import hashlib
import ipaddress
import json
from pathlib import Path
import platform
import re
import socket
import subprocess
from airplay_manual_session import MANIFEST, ROOT, airplay


def dns(*arguments):
    process = subprocess.Popen(['/usr/bin/dns-sd', *arguments], stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True)
    try:
        output, _ = process.communicate(timeout=4)
    except subprocess.TimeoutExpired:
        process.terminate()
        output, _ = process.communicate(timeout=3)
    return output


def main():
    assert platform.system() == 'Darwin', 'macOS only'
    manifest = json.loads(MANIFEST.read_text())
    state = Path(manifest['state_dir'])
    assert airplay(state)['ready'], 'manual receiver is not ready'
    profile = json.loads((state / 'hub/state.json').read_text())
    config = json.loads((state / 'hub/server.json').read_text())
    host = 'neonmix-speaker-' + profile['hub_id'].replace('-', '') + '.local.'
    name = 'NeonMix — ' + config['room_name']
    srv = dns('-L', name, '_airplay._tcp', 'local')
    ports = {int(x) for x in re.findall(r'can be reached at [^\s]+:(\d+)', srv)}
    records = dns('-G', 'v4v6', host)
    addresses = []
    for line in records.splitlines():
        parts = line.split()
        if 'Add' in parts and host in parts:
            address = parts[parts.index(host) + 1]
            # DNSServiceGetAddrInfo prints an unspecified address for a
            # negative AAAA answer. It is not a published address record.
            if not ipaddress.ip_address(address.split('%')[0]).is_unspecified:
                addresses.append(address)
    v6 = [a for a in addresses if ':' in a]
    v4 = {a for a in addresses if ':' not in a}
    checks = []
    if len(ports) == 1:
        for address in v4:
            if ipaddress.ip_address(address).is_loopback:
                continue
            try:
                with socket.create_connection((address, next(iter(ports))), timeout=2) as stream:
                    stream.sendall(b'GET /info RTSP/1.0\r\nCSeq: 1\r\nContent-Length: 0\r\n\r\n')
                    checks.append(stream.recv(256).startswith(b'RTSP/1.0 200'))
            except OSError:
                checks.append(False)
    report = {
        'platform': 'macOS',
        'scope': 'actual mDNS resolution and local TCP requests to advertised IPv4 addresses',
        'limitations': ['not a remote Apple-device playback result', 'IPv6 is disabled for this profile'],
        'ipv6_record_count': len(v6), 'ipv4_address_count': len(v4),
        'srv_port_count': len(ports), 'ipv4_info_checks': checks,
        'passed': not v6 and len(ports) == 1 and bool(checks) and all(checks),
        'hub_sha256': hashlib.sha256((state / 'bin/neonmix-hub').read_bytes()).hexdigest(),
    }
    path = ROOT / 'docs/evidence/airplay/discovery-ipv4-macos.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
