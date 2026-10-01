#!/usr/bin/env python3
"""Administrator transport checks that the desktop owner rejects other UIDs."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
if sys.platform != 'linux' or os.geteuid() != 0:
    raise SystemExit('Requires test-host administrator transport; the audio owner remains non-root')
uid = int(sys.argv[1])
OUT = ROOT/'artifacts/e06-linux-permissions'/time.strftime('%Y%m%d-%H%M%S')
OUT.mkdir(parents=True)
report = {'passed':False,'owner_uid':uid,'scope':'Root rejection and SO_PEERCRED denial; not a full security audit'}
env = os.environ.copy();env['XDG_RUNTIME_DIR']=f'/run/user/{uid}'


def node():
    result = subprocess.run(['setpriv',f'--reuid={uid}',f'--regid={uid}','--init-groups','pw-dump'],env=env,capture_output=True,text=True,check=True)
    nodes=[n for n in json.loads(result.stdout) if n.get('info',{}).get('props',{}).get('node.name')=='neonmix.sink.default']
    assert len(nodes)==1
    return nodes[0]


try:
    original = node();report['original_node']=original
    root = subprocess.run([str(ROOT/'target/release/neonmix-audio'),'virtual-output','--state-directory',str(ROOT/'.local/e06-root-rejected')],cwd=ROOT,env=env,capture_output=True,text=True,timeout=5)
    assert root.returncode != 0 and 'not root' in root.stderr
    report['root_owner_rejected']={'code':root.returncode,'stderr':root.stderr}
    for peer in (0,65534):
        code = f'''import os,socket
os.setgroups([]);os.setgid({peer});os.setuid({peer})
s=socket.socket(socket.AF_UNIX);s.settimeout(3)
s.connect(b'\\0neonmix.virtual-output.owner.{uid}')
try:
    s.sendall(b'Unpersisted forged name');s.shutdown(socket.SHUT_WR);r=s.recv(512)
except (BrokenPipeError,ConnectionResetError):r=b''
assert r!=b'ok'
print(repr(r))
'''
        result=subprocess.run([sys.executable,'-c',code],env=env,capture_output=True,text=True,timeout=5)
        assert result.returncode==0,result.stderr
        report.setdefault('foreign_peers',[]).append({'uid':peer,'response':result.stdout.strip()})
    final = node();assert final['id']==original['id'] and final['info']['props']['node.description']==original['info']['props']['node.description']
    report['node_unchanged']=True;report['passed']=True
except Exception as error:
    report['failure']=repr(error)
finally:
    (OUT/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'passed':report['passed'],'failure':report.get('failure'),'folder':str(OUT)}))
raise SystemExit(0 if report['passed'] else 1)
