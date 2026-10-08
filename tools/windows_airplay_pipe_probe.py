#!/usr/bin/env python3
"""Exercise the production Windows media writer against a private parent pipe.

No audio device or network traffic. Run with tools/dev.ps1, a built
neonmix-airplay-pipe-probe.exe and a project-local --report path.
"""
import argparse
import csv
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    if os.name != 'nt':
        raise SystemExit('Windows native test required')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    report = args.report.resolve()
    report.relative_to(ROOT)
    report.parent.mkdir(parents=True, exist_ok=True)
    k = c.WinDLL('kernel32', use_last_error=True)
    adv = c.WinDLL('advapi32', use_last_error=True)
    class Security(c.Structure):
        _fields_ = [('length', w.DWORD), ('descriptor', c.c_void_p), ('inherit', w.BOOL)]
    adv.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [w.LPCWSTR, w.DWORD, c.POINTER(c.c_void_p), c.c_void_p]
    k.CreateNamedPipeW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, w.DWORD, w.DWORD, w.DWORD, w.DWORD, c.POINTER(Security)]
    k.CreateNamedPipeW.restype = w.HANDLE
    k.PeekNamedPipe.argtypes = [w.HANDLE, c.c_void_p, w.DWORD, c.c_void_p, c.POINTER(w.DWORD), c.c_void_p]
    k.ReadFile.argtypes = [w.HANDLE, c.c_void_p, w.DWORD, c.POINTER(w.DWORD), c.c_void_p]
    k.ConnectNamedPipe.argtypes = [w.HANDLE, c.c_void_p]
    k.CloseHandle.argtypes = [w.HANDLE]
    k.LocalFree.argtypes = [c.c_void_p]
    sid = next(csv.reader(subprocess.check_output(['whoami', '/user', '/fo', 'csv', '/nh'], text=True).strip().splitlines()))[-1]
    assert sid.startswith('S-1-')
    results = []
    for mode in ['throughput', 'stalled', 'stop', 'peer_closed']:
        descriptor = c.c_void_p()
        assert adv.ConvertStringSecurityDescriptorToSecurityDescriptorW(f'O:{sid}D:P(A;;FA;;;{sid})', 1, c.byref(descriptor), None)
        attributes = Security(c.sizeof(Security), descriptor, False)
        address = rf'\\.\pipe\NeonMix.Airplay.v1.{sid}.{uuid.uuid4().hex}'
        pipe = k.CreateNamedPipeW(address, 1 | 0x80000, 1 | 8, 1, 0, 3960, 250, c.byref(attributes))
        k.LocalFree(descriptor)
        assert pipe != w.HANDLE(-1).value, c.get_last_error()
        child = None
        received = 0
        try:
            child = subprocess.Popen([str(args.probe.resolve()), address, mode], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            end = time.monotonic() + 15
            buffer = c.create_string_buffer(65536)
            connected = False
            connected_at = None
            while time.monotonic() < end:
                if not connected:
                    connected = bool(k.ConnectNamedPipe(pipe, None)) or c.get_last_error() == 535
                    if connected:
                        connected_at = time.monotonic()
                if mode == 'peer_closed' and pipe is not None and connected_at is not None and time.monotonic() - connected_at > .05:
                    k.CloseHandle(pipe)
                    pipe = None
                if mode == 'throughput' and connected:
                    available = w.DWORD()
                    if k.PeekNamedPipe(pipe, None, 0, None, c.byref(available), None) and available.value:
                        count = w.DWORD()
                        assert k.ReadFile(pipe, buffer, min(available.value, len(buffer)), c.byref(count), None)
                        assert buffer.raw[:count.value] == b'Z' * count.value
                        received += count.value
                        continue
                if child.poll() is not None:
                    break
                time.sleep(.001)
            if child.poll() is None:
                child.kill()
                raise RuntimeError('pipe probe exceeded 15 seconds')
            stdout, stderr = child.communicate(timeout=2)
            assert child.returncode == 0, stderr
            result = json.loads(stdout)
            result.update(mode=mode, received_bytes=received)
            if mode == 'throughput':
                result['passed'] = result['packets'] == 600 and received == 600 * 3176 and result['elapsed_us'] < 2_000_000
            elif mode == 'stalled':
                result['passed'] = result['packets'] < 2 and 200_000 <= result['elapsed_us'] < 600_000
            elif mode == 'peer_closed':
                result['passed'] = result['packets'] < 3 and result['elapsed_us'] < 200_000
            else:
                result['passed'] = result['packets'] < 2 and result['elapsed_us'] < 200_000
            results.append(result)
        finally:
            if child and child.poll() is None:
                child.kill()
                child.wait(timeout=2)
            if pipe is not None:
                k.CloseHandle(pipe)
    value = dict(passed=all(r['passed'] for r in results), results=results,
                 scope='real protected NamedPipe and production writer; throughput, timeout and cancellation')
    report.write_text(json.dumps(value, indent=2), encoding='utf-8')
    print(json.dumps(value, indent=2))
    return 0 if value['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
