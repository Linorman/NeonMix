#!/usr/bin/env python3
"""Stage and inspect an arm64 HAL installer; never install or change system settings."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--release', action='store_true', help='Require existing Developer ID bundle and installer signing identity')
parser.add_argument('--notary-profile', help='Explicit existing keychain profile; only with --release')
args = parser.parse_args()
if sys.platform != 'darwin':
    parser.error('macOS packaging requires macOS')
if args.notary_profile and not args.release:
    parser.error('Notarization requires explicit release mode')
BUNDLE = ROOT / 'artifacts/macos/NeonMixHAL.driver'
OUT = ROOT / 'artifacts/macos/packages'
OUT.mkdir(parents=True, exist_ok=True)
identifier = 'com.neonmix.audio.hal'
installer_identity = os.environ.get('NEONMIX_INSTALLER_SIGN_IDENTITY', '')
subprocess.run(['codesign', '--verify', '--strict', str(BUNDLE)], check=True)
signature = subprocess.run(['codesign', '-dvvv', str(BUNDLE)], capture_output=True, text=True, check=True)
if args.release:
    if 'Authority=Developer ID Application:' not in signature.stderr or not installer_identity:
        raise SystemExit('Release requires a Developer ID Application signed bundle and NEONMIX_INSTALLER_SIGN_IDENTITY; development signing cannot satisfy this gate')
info = plistlib.loads((BUNDLE / 'Contents/Info.plist').read_bytes())
if info['CFBundleIdentifier'] != 'com.neonmix.audio.driver':
    raise SystemExit('Only the owned NeonMix HAL bundle can be packaged')
version = info['CFBundleVersion']
package = OUT / f'NeonMixHAL-{version}-arm64-{"release" if args.release else "development"}.pkg'
temporary = Path(tempfile.mkdtemp(prefix='hal-package-', dir=ROOT / '.local/tmp'))
phase_warnings = {}

def phase(name, command):
    completed = subprocess.run(command, capture_output=True, text=True)
    (OUT / (package.stem + '-' + name + '.log')).write_text(completed.stdout + completed.stderr)
    if completed.stderr:
        phase_warnings[name] = completed.stderr.strip()
    completed.check_returncode()

try:
    payload = temporary / 'payload'
    payload.mkdir()
    # Package code/resources, not developer-machine quarantine, provenance or ACL metadata.
    phase('copy', ['ditto', '--noextattr', '--noacl', '--noqtn', str(BUNDLE), str(payload / BUNDLE.name)])
    components = temporary / 'components.plist'
    components.write_bytes(plistlib.dumps([{
        'RootRelativeBundlePath': BUNDLE.name,
        'BundleIsRelocatable': False,
        'BundleHasStrictIdentifier': True,
        'BundleIsVersionChecked': args.release,
        'BundleOverwriteAction': 'upgrade',
    }]))
    component = temporary / 'NeonMixHAL-component.pkg'
    phase('component', ['pkgbuild', '--root', str(payload), '--component-plist', str(components),
                    '--identifier', identifier, '--version', version, '--ownership', 'recommended',
                    '--install-location', '/Library/Audio/Plug-Ins/HAL', str(component)])
    distribution = temporary / 'Distribution.xml'
    # A reboot conclusively reloads the newly installed driver. The development
    # activation script can use the shorter explicit audio-service restart path.
    distribution.write_text(f'''<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
 <title>NeonMix Virtual Output</title>
 <options customize="never" require-scripts="false" hostArchitectures="arm64"/>
 <domains enable_localSystem="true" enable_currentUserHome="false" enable_anywhere="false"/>
 <allowed-os-versions><os-version min="14.6"/></allowed-os-versions>
 <choices-outline><line choice="hal"/></choices-outline>
 <choice id="hal" visible="false"><pkg-ref id="{identifier}"/></choice>
 <pkg-ref id="{identifier}" version="{escape(version)}" onConclusion="RequireRestart">NeonMixHAL-component.pkg</pkg-ref>
</installer-gui-script>
''')
    command = ['productbuild', '--distribution', str(distribution), '--package-path', str(temporary)]
    if args.release:
        command += ['--sign', installer_identity, '--timestamp']
    command += [str(package)]
    phase('product', command)
    expanded = temporary / 'expanded'
    phase('expand', ['pkgutil', '--expand-full', str(package), str(expanded)])
    boms = list(expanded.rglob('Bom'))
    if len(boms) != 1:
        raise RuntimeError('Installer must contain one payload manifest')
    manifest = subprocess.check_output(['lsbom', str(boms[0])], text=True)
    (OUT / (package.stem + '-payload-manifest.log')).write_text(manifest)
    for line in manifest.splitlines():
        columns = line.split('\t')
        if len(columns) < 3 or columns[2] != '0/0':
            raise RuntimeError('Payload installation ownership must be root:wheel')
        if columns[0] not in ('.', './NeonMixHAL.driver') and not columns[0].startswith('./NeonMixHAL.driver/'):
            raise RuntimeError('Payload includes files outside the owned HAL bundle')
        mode = int(columns[1], 8)
        if mode & 0o022 or mode & 0o170000 not in (0o100000, 0o040000):
            raise RuntimeError('Payload must contain only protected regular files/directories')
    binaries = list(expanded.rglob('NeonMixHAL.driver/Contents/MacOS/NeonMixHAL'))
    if len(binaries) != 1:
        raise RuntimeError('Installer must contain exactly one owned HAL bundle')
    extracted_bundle = binaries[0].parents[2]
    subprocess.run(['codesign', '--verify', '--strict', str(extracted_bundle)], check=True)
    subprocess.run(['lipo', '-verify_arch', 'arm64', str(binaries[0])], check=True)
    source_hash = hashlib.sha256((BUNDLE / 'Contents/MacOS/NeonMixHAL').read_bytes()).hexdigest()
    payload_hash = hashlib.sha256(binaries[0].read_bytes()).hexdigest()
    if source_hash != payload_hash:
        raise RuntimeError('Installer payload differs from reviewed bundle')
    signature_check = subprocess.run(['pkgutil', '--check-signature', str(package)], capture_output=True, text=True)
    (OUT / (package.stem + '-signature.log')).write_text(signature_check.stdout + signature_check.stderr)
    if args.release:
        signature_check.check_returncode()
        if 'Developer ID Installer:' not in signature_check.stdout + signature_check.stderr:
            raise RuntimeError('Release installer does not carry a Developer ID Installer signature')
    if args.notary_profile:
        subprocess.run(['xcrun', 'notarytool', 'submit', str(package), '--keychain-profile', args.notary_profile, '--wait'], check=True)
        subprocess.run(['xcrun', 'stapler', 'staple', str(package)], check=True)
        subprocess.run(['xcrun', 'stapler', 'validate', str(package)], check=True)
        subprocess.run(['spctl', '--assess', '--type', 'install', '--verbose', str(package)], check=True)
    report = {'passed': True, 'scope': 'macOS installer staging and extracted payload verification; no system installation',
              'release_signed': args.release, 'notarization_executed': bool(args.notary_profile),
              'restart_required': True, 'package': str(package.relative_to(ROOT)),
              'payload_ownership': 'root:wheel',
              'package_sha256': hashlib.sha256(package.read_bytes()).hexdigest(),
              'source_binary_sha256': source_hash, 'payload_binary_sha256': payload_hash,
              'phase_warnings': phase_warnings}
    (OUT / (package.stem + '.json')).write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))
finally:
    shutil.rmtree(temporary)
