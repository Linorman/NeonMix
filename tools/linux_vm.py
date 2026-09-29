#!/usr/bin/env python3
"""Project-contained Ubuntu x86_64 runtime lab; not a physical-device acceptance substitute.
QEMU is built into .local/qemu-build and all guest state stays in .local/linux-vm.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
VM = ROOT / '.local/linux-vm'
OUT = ROOT / 'artifacts/linux-vm'
QEMU = ROOT / '.local/qemu-build/qemu-10.1.5/build/qemu-system-x86_64'
QEMU_IMG = QEMU.with_name('qemu-img')
PORT = 22282
VM.mkdir(parents=True, exist_ok=True)
OUT.mkdir(parents=True, exist_ok=True)


def qmp(command):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(3)
        connection.connect(str(VM / 'qmp.sock'))
        reader = connection.makefile('rb')
        json.loads(reader.readline())
        for cmd in ['qmp_capabilities', command]:
            connection.sendall((json.dumps({'execute': cmd}) + '\n').encode())
            while True:
                response = json.loads(reader.readline())
                if 'return' in response or 'error' in response:
                    break
        return response


def ssh_command():
    return ['ssh', '-F', '/dev/null', '-i', str(VM / 'id_ed25519'), '-p', str(PORT),
            '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=8', '-o', 'IdentitiesOnly=yes',
            '-o', 'StrictHostKeyChecking=accept-new', '-o', f'UserKnownHostsFile={VM / "known_hosts"}',
            'ubuntu@127.0.0.1']


def start():
    try:
        print(json.dumps(qmp('query-status')))
        return
    except (OSError, ValueError):
        pass
    if not QEMU.exists() or not (VM / 'SHA256SUMS').exists():
        raise RuntimeError('Build the project-local QEMU and download the pinned Ubuntu image/checksums first')
    expected = next(line.split()[0] for line in (VM / 'SHA256SUMS').read_text().splitlines()
                    if line.split()[-1].lstrip('*') == 'ubuntu-24.04-server-cloudimg-amd64.img')
    with (VM / 'base.img').open('rb') as image:
        digest = hashlib.file_digest(image, 'sha256').hexdigest()
    if digest != expected:
        raise RuntimeError('Ubuntu image checksum mismatch; refusing to start the VM')
    if not (VM / 'id_ed25519').exists():
        subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-C', 'neonmix-local-lab', '-f', str(VM / 'id_ed25519')], check=True)
    seed = VM / 'seed'
    seed.mkdir(exist_ok=True)
    public_key = (VM / 'id_ed25519.pub').read_text().strip()
    (seed / 'meta-data').write_text('instance-id: neonmix-e01-noble-x64\nlocal-hostname: neonmix-lab\n')
    (seed / 'user-data').write_text(f'''#cloud-config
users:
  - default
ssh_pwauth: false
ssh_authorized_keys:
  - {public_key}
package_update: true
packages:
  - pipewire
  - pipewire-bin
  - wireplumber
  - dbus-user-session
  - python3
runcmd:
  - [loginctl, enable-linger, ubuntu]
  - [touch, /var/lib/neonmix-lab-provisioned]
''')
    if not (VM / 'instance.qcow2').exists():
        subprocess.run([str(QEMU_IMG), 'create', '-f', 'qcow2', '-F', 'qcow2', '-b', str(VM / 'base.img'), str(VM / 'instance.qcow2'), '16G'], check=True)
    subprocess.run(['hdiutil', 'makehybrid', '-quiet', '-iso', '-joliet', '-default-volume-name', 'cidata',
                    '-o', str(VM / 'seed.iso'), str(seed), '-ov'], check=True)
    command = [str(QEMU), '-machine', 'q35', '-accel', 'tcg,thread=multi', '-cpu', 'max', '-smp', '2', '-m', '2048',
               '-drive', f'file={VM / "instance.qcow2"},if=virtio,format=qcow2',
               '-drive', f'file={VM / "seed.iso"},media=cdrom,readonly=on',
               '-netdev', f'user,id=labnet,hostfwd=tcp:127.0.0.1:{PORT}-:22', '-device', 'virtio-net-pci,netdev=labnet',
               '-display', 'none', '-monitor', 'none', '-serial', f'file:{OUT / "serial.log"}',
               '-qmp', f'unix:{VM / "qmp.sock"},server=on,wait=off', '-no-reboot']
    with (OUT / 'qemu.stderr').open('w') as stderr:
        child = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.DEVNULL, stdout=stderr, stderr=stderr, start_new_session=True)
    (VM / 'pid').write_text(str(child.pid))
    (OUT / 'vm.json').write_text(json.dumps({'pid': child.pid, 'image_sha256': digest,
        'image_url': 'https://cloud-images.ubuntu.com/releases/noble/release-20260911/ubuntu-24.04-server-cloudimg-amd64.img',
        'qemu_version': subprocess.check_output([str(QEMU), '--version'], text=True).splitlines()[0],
        'architecture': 'x86_64', 'accelerator': 'TCG', 'scope': 'virtual machine runtime, no physical audio hardware'}, indent=2)+'\n')
    print(json.dumps({'started_pid': child.pid, 'ssh_port': PORT}))


parser = argparse.ArgumentParser()
parser.add_argument('action', choices=['start', 'status', 'stop', 'configure', 'ssh', 'copy'])
parser.add_argument('rest', nargs=argparse.REMAINDER)
args = parser.parse_args()
if args.action == 'start':
    start()
elif args.action == 'status':
    try:
        status = qmp('query-status')
    except (OSError, ValueError) as error:
        status = {'running': False, 'error': str(error)}
    print(json.dumps(status))
    if 'return' in status:
        result = subprocess.run([*ssh_command(), 'test -e /var/lib/neonmix-lab-provisioned && uname -m && pipewire --version'], capture_output=True, text=True)
        print(json.dumps({'ssh_exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}))
elif args.action == 'stop':
    print(json.dumps(qmp('system_powerdown')))
elif args.action == 'configure':
    command = """mkdir -p ~/neonmix/.local/config/pipewire/pipewire.conf.d &&
    printf '%s\n' 'context.properties = { default.clock.rate = 48000 default.clock.allowed-rates = [ 44100 48000 96000 ] }' > ~/neonmix/.local/config/pipewire/pipewire.conf.d/10-neonmix.conf &&
    systemctl --user set-environment XDG_CONFIG_HOME=/home/ubuntu/neonmix/.local/config &&
    systemctl --user restart pipewire.service wireplumber.service"""
    raise SystemExit(subprocess.call([*ssh_command(), command]))
elif args.action == 'ssh':
    if not args.rest:
        raise RuntimeError('Provide one remote shell command')
    raise SystemExit(subprocess.call([*ssh_command(), *args.rest]))
elif args.action == 'copy':
    if len(args.rest) != 2:
        raise RuntimeError('copy requires a local source and remote destination')
    # tar over the same SSH transport avoids SCP default-protocol differences.
    source = Path(args.rest[0]).resolve()
    if not source.is_relative_to(ROOT):
        raise RuntimeError('The copied source must be inside the project')
    destination = args.rest[1]
    with source.open('rb') as data:
        raise SystemExit(subprocess.call([*ssh_command(), 'mkdir -p ' + shlex.quote(str(Path(destination).parent)) + ' && cat > ' + shlex.quote(destination)], stdin=data))
