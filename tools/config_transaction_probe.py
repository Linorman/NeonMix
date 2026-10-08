#!/usr/bin/env python3
"""Kill the real transaction writer at every filesystem seam, then restart recovery.
All private fixture documents are removed; public evidence contains only stages/counts.
"""
import argparse,hashlib,json,os,shutil,signal,subprocess,time,uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def run(binary,directory,*args):
    return subprocess.run([str(binary),'--directory',str(directory),*args],check=True,capture_output=True,text=True)
def digest(data):return hashlib.sha256(data).hexdigest()
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--probe',type=Path,required=True);parser.add_argument('--report',type=Path,required=True);args=parser.parse_args()
    binary=args.probe.resolve();binary.relative_to(ROOT);destination=args.report.resolve();destination.relative_to(ROOT)
    base=ROOT/'.local/tmp'/('config-transaction-'+uuid.uuid4().hex);base.mkdir(mode=0o700)
    result=dict(passed=False,scope='actual private journal/files, independent writer SIGKILL and fresh recovery; no power-loss or Windows filesystem claim',probe_sha256=digest(binary.read_bytes()),cases=[])
    process=None
    try:
        complete=base/'complete';run(binary,complete,'--init');total=json.loads(run(binary,complete).stdout)['boundaries']
        for index in range(1,total+1):
            directory=base/str(index);run(binary,directory,'--init')
            old_profile=json.loads((directory/'server.json').read_text());old_state=json.loads((directory/'state.json').read_text())
            identity={key:old_profile[key] for key in ['certificate','private_key_ref','admin_token_ref']}
            process=subprocess.Popen([str(binary),'--directory',str(directory),'--stop-at',str(index)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            # The child flushes its deterministic seam and waits on our pipe.
            # No sleep is used to choose the kill window.
            barrier=json.loads(process.stdout.readline());assert barrier['event']=='barrier' and barrier['index']==index
            process.kill();process.wait(timeout=3);process.stdin.close();process.stdout.close();process.stderr.close();process=None
            recovered=json.loads(run(binary,directory,'--recover').stdout)
            profile=json.loads((directory/'server.json').read_text());state=json.loads((directory/'state.json').read_text())
            new=recovered['new'];assert profile['output']==state['output']['id']==('new-output' if new else 'old-output')
            assert profile['room_name']==('New room' if new else 'Old room')
            assert {key:profile[key] for key in identity}==identity
            for key in ['hub_id','devices','credentials','preferences']:assert state[key]==old_state[key]
            assert state['revision']==old_state['revision']+int(new)
            assert not list(directory.glob('*.neonmix-transaction.json'))
            assert not list(directory.rglob('.neonmix-config-*.tmp')),'pre-journal staging file leaked'
            # Re-running recovery is side-effect free and doesn't bump revision.
            assert json.loads(run(binary,directory,'--recover').stdout)==recovered
            result['cases'].append(dict(index=index,stage=barrier['stage'],new=new,revision=recovered['revision'],passed=True))
        result.update(passed=True,boundaries=total)
    except Exception as error:result.update(failure_type=type(error).__name__,failure=str(error))
    finally:
        if process:
            if process.poll() is None:process.kill();process.wait(timeout=3)
            for pipe in [process.stdin,process.stdout,process.stderr]:pipe.close()
        shutil.rmtree(base);result['fixtures_removed']=True
        destination.parent.mkdir(parents=True,exist_ok=True);destination.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(dict(passed=result['passed'],cases=len(result['cases']),failure=result.get('failure'))));return 0 if result['passed'] else 1
if __name__=='__main__':raise SystemExit(main())
