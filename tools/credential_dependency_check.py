#!/usr/bin/env python3
"""Require native credential backends to stay outside default product graphs."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    for package in ['neonmix-identity', 'neonmix-hub', 'neonmix-desktop-service', 'neonmix-desktop']:
        tree = subprocess.run(['cargo', 'tree', '--locked', '-p', package],
                              cwd=ROOT, capture_output=True, text=True, check=True).stdout
        if 'keyring v' in tree:
            raise RuntimeError('native credential dependency in ' + package)
    print('default identity, Hub, background and desktop graphs exclude keyring')


if __name__ == '__main__':
    main()
