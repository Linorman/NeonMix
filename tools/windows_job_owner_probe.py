#!/usr/bin/env python3
"""Kill only this probe's owned parent; verify its pinned job children exit."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import threading

ROOT=Path(__file__).resolve().parents[1]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe',type=Path,required=True)
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args()
    assert os.name=='nt'
    binary=args.probe.resolve();binary.relative_to(ROOT)
    report=args.report.resolve();report.relative_to(ROOT)
    kernel=ctypes.WinDLL('kernel32',use_last_error=True)
    kernel.OpenProcess.argtypes=[wintypes.DWORD,wintypes.BOOL,wintypes.DWORD]
    kernel.OpenProcess.restype=wintypes.HANDLE
    kernel.WaitForSingleObject.argtypes=[wintypes.HANDLE,wintypes.DWORD]
    kernel.TerminateProcess.argtypes=[wintypes.HANDLE,wintypes.UINT]
    kernel.CloseHandle.argtypes=[wintypes.HANDLE]
    process=subprocess.Popen([str(binary),'--job-owner'],stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                             creationflags=subprocess.CREATE_NO_WINDOW,text=True)
    events=queue.Queue()
    def reader():
        for line in process.stdout:events.put(json.loads(line))
    thread=threading.Thread(target=reader,daemon=True);thread.start()
    handles=[]
    try:
        for _ in range(3):
            event=events.get(timeout=10)
            if event['event'] in ['job_child','descendant']:
                handle=kernel.OpenProcess(0x100001,False,event['pid'])
                assert handle,'could not pin owned child'
                handles.append(handle)
        assert len(handles)==2
        process.kill();process.wait(timeout=5)
        assert all(kernel.WaitForSingleObject(handle,2000)==0 for handle in handles),'job left a process alive'
        result={'passed':True,'owned_children':2,'parent_abnormal_exit':True,'children_exit_verified_by_handle':True,
                'probe_sha256':hashlib.sha256(binary.read_bytes()).hexdigest()}
        report.parent.mkdir(parents=True,exist_ok=True)
        report.write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
        print(json.dumps(result,indent=2))
    finally:
        if process.poll() is None:process.kill();process.wait(timeout=5)
        for handle in handles:
            if kernel.WaitForSingleObject(handle,0)!=0:
                kernel.TerminateProcess(handle,1);kernel.WaitForSingleObject(handle,2000)
            kernel.CloseHandle(handle)
        thread.join(2)
        process.stdout.close();process.stderr.close()


if __name__=='__main__':main()
