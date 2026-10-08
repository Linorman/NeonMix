#!/usr/bin/env python3
"""Kill a real project background; verify its synthetic owned tree and another instance.

No audio device, no real credentials. All artifacts and fixture paths are local.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import time
import uuid

ROOT=Path(__file__).resolve().parents[1]

def rpc(state,request):
    lifecycle=request['type'] in ['status','hub_stop','sender_stop','shutdown','lifecycle_operation','lifecycle_stop']
    endpoint=state/('lifecycle.sock' if lifecycle else 'ipc.sock')
    with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as connection:
        connection.settimeout(15);connection.connect(str(endpoint))
        body=json.dumps(dict(version=1,request=request)).encode()
        connection.sendall(struct.pack('>I',len(body))+body)
        def read(count):
            result=b''
            while len(result)<count:
                chunk=connection.recv(count-len(result))
                if not chunk: raise EOFError('IPC closed')
                result+=chunk
            return result
        length=struct.unpack('>I',read(4))[0]
        assert length<=262144
        return json.loads(read(length))

def alive(pid):
    try: os.kill(pid,0)
    except ProcessLookupError: return False
    state=subprocess.run(['ps','-p',str(pid),'-o','stat='],capture_output=True,text=True,check=False).stdout.strip()
    return bool(state) and not state.startswith('Z')

def wait_state(state):
    deadline=time.monotonic()+8
    while time.monotonic()<deadline:
        try:
            reply=rpc(state,dict(type='status'))
            if reply['ok']: return reply['data']
        except (OSError,EOFError): pass
        time.sleep(.02)
    raise TimeoutError('background not ready')

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--background',type=Path,required=True)
    parser.add_argument('--probe',type=Path,required=True)
    parser.add_argument('--guardian',type=Path)
    parser.add_argument('--stop-instead-of-kill',action='store_true',help='Inspect the same supervisor result while its real background remains alive')
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args();assert os.name=='posix'
    for path in [args.background,args.probe,args.report,*([args.guardian] if args.guardian else [])]:path.resolve().relative_to(ROOT)
    directory=ROOT/'.local/tmp'/('unix-owner-'+uuid.uuid4().hex);directory.mkdir(mode=0o700)
    package=directory/'bin';package.mkdir(mode=0o700)
    shutil.copy2(args.background,package/'neonmix-background')
    if args.guardian:shutil.copy2(args.guardian,package/'neonmix-guardian')
    helper=package/'neonmix-hub'
    # exec retains the direct child's identity; the probe owns one descendant.
    import shlex
    helper.write_text('#!/bin/sh\nexec '+shlex.quote(str(args.probe.resolve()))+' --ignore-stop --spawn-descendant "$@"\n');helper.chmod(0o700)
    audit=directory/'guardian-audit';audit.mkdir(mode=0o700)
    processes=[];media=[];guardians=[];logs=[];report=dict(passed=False,scope='real background, synthetic Hub/probe tree; no audio or Apple devices',binary_sha256={path.name:hashlib.sha256(path.read_bytes()).hexdigest() for path in [args.background,args.probe,*([args.guardian] if args.guardian else [])]})
    try:
        states=[]
        for index in range(2):
            state=directory/str(index);state.mkdir(mode=0o700);hub=state/'hub';hub.mkdir(mode=0o700)
            profile=dict(version=2,credential_store='file',room_name='Owner fixture',output='fixture',state_path='state.json',certificate='fixture-public-certificate',private_key_ref=str(uuid.uuid4()),admin_token_ref=str(uuid.uuid4()))
            config=hub/'server.json';config.write_text(json.dumps(profile));config.chmod(0o600)
            (hub/'state.json').write_text('{}');(hub/'state.json').chmod(0o600)
            log=(directory/f'{index}.log').open('wb');logs.append(log)
            process=subprocess.Popen([str(package/'neonmix-background'),'--state-dir',str(state)],stdout=log,stderr=log,env=dict(os.environ,NEONMIX_GUARDIAN_AUDIT_DIR=str(audit)));processes.append(process);states.append(state)
            wait_state(state);started=rpc(state,dict(type='hub_start'));assert started['ok'],started
            snapshot=rpc(state,dict(type='status'))['data'];assert snapshot['hub']['ready']
            pid=snapshot['hub']['pid'];descendants=subprocess.run(['pgrep','-P',str(pid)],capture_output=True,text=True,check=False).stdout.split()
            assert len(descendants)==1,'fixture has no owned descendant'
            tree=[pid,int(descendants[0])];media.append(tree)
            guardian=snapshot['hub'].get('owner_pid')
            guardians.append(guardian if guardian!=pid else None)
            # Synthetic run key plus durable profile sentinel. No credential
            # bytes are used, and cleanup must affect only this instance.
            runtime=hub/'airplay';runtime.mkdir(mode=0o700)
            key=runtime/('runtime-key-'+str(uuid.uuid4()));key.write_text('fixture runtime key');key.chmod(0o600)
        # Let the live owner observe newly created run-key metadata.
        time.sleep(.2)
        if args.stop_instead_of_kill:
            accepted=rpc(states[0],dict(type='hub_stop'));assert accepted['ok']
        else:
            processes[0].kill();processes[0].wait(timeout=3)
        started=time.monotonic();remaining=media[0]
        while time.monotonic()-started<8:
            remaining=[pid for pid in media[0] if alive(pid)]
            if not remaining and (not guardians[0] or not alive(guardians[0])):break
            assert processes[1].poll() is None
            time.sleep(.02)
        report.update(owned_tree_pids=media[0],owned_tree_remaining=remaining,elapsed_ms=round((time.monotonic()-started)*1000),other_instance_pid=processes[1].pid,other_instance_tree_alive=all(alive(pid) for pid in media[1]))
        assert not remaining,'background kill left synthetic media running beyond 8 seconds'
        assert report['other_instance_tree_alive'],'other instance was affected'
        if args.stop_instead_of_kill:
            report['supervisor_status']=rpc(states[0],dict(type='status'))['data']['hub']
        if args.guardian:
            report['owned_runtime_keys_removed']=not list((states[0]/'hub/airplay').glob('runtime-key-*'))
            report['other_runtime_key_preserved']=len(list((states[1]/'hub/airplay').glob('runtime-key-*')))==1
            report['profiles_preserved']=all((state/'hub/server.json').is_file() for state in states)
            assert report['owned_runtime_keys_removed'] and report['other_runtime_key_preserved'] and report['profiles_preserved']
        report['passed']=True
    except Exception as error:
        report.update(failure_type=type(error).__name__,failure=str(error))
    finally:
        for process in processes:
            if process.poll() is None:process.kill();process.wait(timeout=3)
        # Only PIDs freshly returned by these two private fixture instances.
        for tree in media:
            for pid in tree:
                if alive(pid):
                    try:os.kill(pid,signal.SIGKILL)
                    except ProcessLookupError:pass
        deadline=time.monotonic()+3
        while any(pid and alive(pid) for pid in guardians) and time.monotonic()<deadline:time.sleep(.02)
        report['guardians_exited']=all(not pid or not alive(pid) for pid in guardians)
        report['supervisor_receipts']=[json.loads(path.read_text()) for path in audit.glob('*.json')]
        for log in logs:log.close()
        shutil.rmtree(directory)
        report['fixtures_removed']=True
        destination=args.report.resolve();destination.parent.mkdir(parents=True,exist_ok=True);destination.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2));return 0 if report['passed'] else 1

if __name__=='__main__':raise SystemExit(main())
