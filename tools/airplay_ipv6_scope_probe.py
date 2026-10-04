#!/usr/bin/env python3
"""macOS: validate SpeakerPublisher's IPv6 mDNS scope with an isolated fixture.

Run with tools/dev. Uses a unique receiver identity and hostname; never starts
the AirPlay worker or touches an existing manual-session receiver. The saved
report contains counts only, without IP addresses or receiver credentials.
"""
import hashlib
import ipaddress
import json
from pathlib import Path
import platform
import re
import socket
import struct
import subprocess
import tempfile
import time
import uuid


ROOT = Path(__file__).resolve().parents[1]
REPORT = ROOT / 'docs/evidence/airplay/discovery-ipv6-scope-macos.json'


def dns(interface, *arguments):
    return subprocess.run(
        ['/usr/bin/dns-sd', '-t', '4', '-i', interface, *arguments],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=8,
        check=True,
    ).stdout


def interface_addresses():
    text = subprocess.check_output(['/sbin/ifconfig'], text=True)
    owned = {}
    current = None
    for line in text.splitlines():
        if line and not line[0].isspace():
            current = line.split(':', 1)[0]
            owned[current] = set()
        match = re.search(r'\binet6?\s+(\S+)', line)
        if match and current:
            owned[current].add(ipaddress.ip_address(match[1].split('%')[0]))
    return owned


def resolve_addresses(text, host):
    results = set()
    for line in text.splitlines():
        parts = line.split()
        if 'Add' not in parts or host not in parts:
            continue
        # DNSServiceGetAddrInfo's unspecified negative answers are not records.
        address = parts[parts.index(host) + 1]
        raw, _, scope = address.partition('%')
        ip = ipaddress.ip_address(raw)
        if not ip.is_unspecified:
            results.add((ip, scope))
    return results


def aaaa_records(packet, transaction, host):
    if len(packet) < 12:
        return []
    ident, flags, questions, answers, authorities, additionals = struct.unpack('!6H', packet[:12])
    if ident != transaction or not flags & 0x8000:
        return []
    def read_name(offset, visited=None):
        visited = set() if visited is None else visited
        labels = []
        while packet[offset]:
            if offset in visited:
                raise ValueError('cyclic DNS name')
            visited.add(offset)
            if packet[offset] & 0xc0 == 0xc0:
                target = struct.unpack('!H', packet[offset:offset + 2])[0] & 0x3fff
                suffix, _ = read_name(target, visited)
                return '.'.join(labels + [suffix]), offset + 2
            size = packet[offset]
            labels.append(packet[offset + 1:offset + 1 + size].decode('latin1'))
            offset += size + 1
        return '.'.join(labels), offset + 1
    offset = 12
    for _ in range(questions):
        _, offset = read_name(offset)
        offset += 4
    addresses = []
    for _ in range(answers + authorities + additionals):
        name, offset = read_name(offset)
        rrtype, _, _, length = struct.unpack('!HHIH', packet[offset:offset + 10])
        offset += 10
        if name.lower() == host.rstrip('.').lower() and rrtype == 28 and length == 16:
            addresses.append(ipaddress.ip_address(packet[offset:offset + length]))
        offset += length
    return addresses


def wire_aaaa(interface, host, owned):
    # Multicast replies retain the daemon's configured source interface.
    # Legacy unicast would instead let OS routing select a different source
    # address when querying locally between two NICs on the same LAN.
    source = next(ip for ip in owned[interface] if ip.version == 4)
    transaction = 0
    encoded_name = b''.join(bytes([len(label)]) + label.encode() for label in host.rstrip('.').split('.')) + b'\0'
    query = struct.pack('!6H', transaction, 0, 1, 0, 0, 0) + encoded_name + struct.pack('!HH', 28, 1)
    replies = {}
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as stream:
        stream.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        stream.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
        stream.bind(('0.0.0.0', 5353))
        stream.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, source.packed)
        stream.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
                          socket.inet_aton('224.0.0.251') + source.packed)
        stream.settimeout(0.3)
        until = time.monotonic() + 3
        stream.sendto(query, ('224.0.0.251', 5353))
        while time.monotonic() < until:
            try:
                packet, peer = stream.recvfrom(65535)
            except socket.timeout:
                continue
            addresses = aaaa_records(packet, transaction, host)
            source_ip = ipaddress.ip_address(peer[0])
            owners = [name for name, ips in owned.items() if source_ip in ips]
            if not addresses or not owners:
                continue
            for owner in owners:
                entry = replies.setdefault(owner, {'reply_count': 0, 'link_local_record_count': 0,
                                                   'foreign_link_local_record_count': 0})
                entry['reply_count'] += 1
                entry['link_local_record_count'] += sum(ip.is_link_local for ip in addresses)
                entry['foreign_link_local_record_count'] += sum(
                    ip.is_link_local and ip not in owned[owner] for ip in addresses)
    return replies


def main():
    assert platform.system() == 'Darwin', 'macOS only'
    owned = interface_addresses()
    # Require both the Wi-Fi link and another real Ethernet link for this
    # multi-interface regression; do not silently weaken its acceptance.
    interfaces = [name for name in owned if name.startswith('en')
                  and any(ip.version == 6 and ip.is_link_local for ip in owned[name])]
    assert 'en0' in interfaces and len(interfaces) >= 2, 'two IPv6 LAN interfaces required'
    receiver = uuid.uuid4()
    name = 'NeonMix IPv6 Scope ' + receiver.hex[:8]
    host = 'neonmix-speaker-' + receiver.hex + '.local.'
    with tempfile.TemporaryDirectory(prefix='airplay-ipv6-scope-', dir=ROOT / '.local/tmp') as tmp:
        fixture = Path(tmp)
        (fixture / 'src').mkdir()
        (fixture / 'Cargo.toml').write_text(
            '[package]\nname = "neonmix-scoped-discovery-fixture"\nversion = "0.1.0"\nedition = "2024"\n'
            '[workspace]\n[dependencies]\nuuid = "1"\n'
            f'neonmix-identity = {{ path = "{ROOT / "crates/identity"}" }}\n')
        # Keep resolved dependency versions identical to the project build.
        (fixture / 'Cargo.lock').write_bytes((ROOT / 'Cargo.lock').read_bytes())
        (fixture / 'src/main.rs').write_text('''use neonmix_identity::speaker::SpeakerPublisher;
use std::io::{self, BufRead};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let _publisher = SpeakerPublisher::new(args[1].parse().unwrap(), &args[2],
        "0.0.0.0:7000".parse().unwrap(), &"0".repeat(64), "0x5A7FFFF7,0x1E").unwrap();
    println!("ready");
    let _ = io::stdin().lock().lines().next();
}
''')
        subprocess.run(['cargo', 'build', '--offline', '--manifest-path', str(fixture / 'Cargo.toml'),
                        '--target-dir', str(fixture / 'target')], check=True)
        binary = fixture / 'target/debug/neonmix-scoped-discovery-fixture'
        binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
        process = subprocess.Popen([str(binary), str(receiver), name], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            assert process.stdout.readline().strip() == 'ready', 'publisher failed to start'
            checks = []
            for interface in interfaces:
                addresses = resolve_addresses(dns(interface, '-G', 'v4v6', host), host)
                link_local = [(ip, scope) for ip, scope in addresses if ip.version == 6 and ip.is_link_local]
                invalid = [(ip, scope) for ip, scope in link_local
                           if ip not in owned[interface]
                           or scope not in {interface, str(socket.if_nametoindex(interface))}]
                airplay = dns(interface, '-L', name, '_airplay._tcp', 'local')
                airplay_resolved = bool(re.search(r'can be reached at ' + re.escape(host) + r':7000\b', airplay))
                wire = wire_aaaa(interface, host, owned)
                # Two NICs on the same physical LAN can receive each other's
                # multicast. Native callbacks label the receiving scope. Use
                # actual reply source ownership to test the publisher itself.
                unsafe_native = sum(not any(ip in owned[n] for n in interfaces) for ip, _ in link_local)
                wire_passed = interface in wire and all(
                    entry['link_local_record_count'] > 0 and entry['foreign_link_local_record_count'] == 0
                    for entry in wire.values())
                checks.append({
                    'interface': interface,
                    'ipv4_address_count': sum(ip.version == 4 for ip, _ in addresses),
                    'ipv6_address_count': sum(ip.version == 6 for ip, _ in addresses),
                    'link_local_record_count': len(link_local),
                    'native_callback_different_lan_interface_count': len(invalid),
                    'native_callback_non_lan_link_local_count': unsafe_native,
                    'wire_aaaa_replies_by_source_interface': wire,
                    'airplay_srv_resolved': airplay_resolved,
                    'passed': bool(link_local) and unsafe_native == 0 and airplay_resolved and wire_passed,
                })
        finally:
            # Graceful Drop sends goodbye records; only this fixture is stopped.
            process.communicate(input='stop\n', timeout=8)
    report = {
        'platform': 'macOS',
        'scope': 'actual SpeakerPublisher native DNS-SD resolution and IPv4 multicast AAAA replies attributed to source-interface ownership',
        'limitations': ['independent discovery fixture; no RTSP or audio playback',
                        'no Windows or Linux runtime verification',
                        'global/ULA subnet semantics covered by unit tests',
                        'same-LAN NICs can receive each other multicast; native callback scope identifies reception rather than advertisement origin'],
        'fixture_sha256': binary_hash,
        'interfaces': checks,
        'passed': all(check['passed'] for check in checks),
    }
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
