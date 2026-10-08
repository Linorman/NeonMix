#!/usr/bin/env python3
"""Block the guardian log consumer; its private Stop and tree cleanup still finish."""
import argparse,hashlib,json,os,signal,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def children(pid):
    return [int(value) for value in subprocess.run(['pgrep','-P',str(pid)],capture_output=True,text=True,check=False).stdout.split()]
def alive(pid):
    try:os.kill(pid,0)
    except ProcessLookupError:return False
    state=subprocess.run(['ps','-p',str(pid),'-o','stat='],capture_output=True,text=True,check=False).stdout.strip()
    return bool(state) and not state.startswith('Z')
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--guardian',type=Path,required=True);parser.add_argument('--probe',type=Path,required=True);parser.add_argument('--report',type=Path,required=True);args=parser.parse_args()
    for path in [args.guardian,args.probe,args.report]:path.resolve().relative_to(ROOT)
    process=subprocess.Popen([str(args.guardian),'--child',str(args.probe.resolve()),'--pipe-child','--','serve','--managed-control-stdin','--spawn-descendant','--flood-log','--ignore-stop'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    report=dict(passed=False,scope='real supervisor/private pipe and synthetic log flood; no media or hardware',binary_sha256={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [args.guardian,args.probe]});owned=[]
    try:
        deadline=time.monotonic()+3
        while time.monotonic()<deadline:
            roots=children(process.pid)
            if len(roots)==1:
                descendants=children(roots[0])
                if len(descendants)==1:owned=roots+descendants;break
            time.sleep(.01)
        assert len(owned)==2
        time.sleep(.5);assert all(alive(pid) for pid in owned),'fixture exited before blocked-consumer Stop'
        started=time.monotonic();process.stdin.write(b'{"version":1,"type":"stop"}\n');process.stdin.flush();process.stdin.close()
        # Do not read stdout/stderr: a completely full parent log pipe is the fault.
        exit_code=process.wait(timeout=9)
        report.update(exit_code=exit_code,elapsed_ms=round((time.monotonic()-started)*1000),owned_tree_pids=owned,owned_tree_remaining=[pid for pid in owned if alive(pid)])
        assert not report['owned_tree_remaining'];report['passed']=True
    except Exception as error:report.update(failure_type=type(error).__name__,failure=str(error))
    finally:
        if process.poll() is None:process.kill();process.wait(timeout=3)
        for pid in owned:
            if alive(pid):
                try:os.kill(pid,signal.SIGKILL)
                except ProcessLookupError:pass
        process.stdout.close();process.stderr.close()
        if process.stdin and not process.stdin.closed:process.stdin.close()
        args.report.resolve().parent.mkdir(parents=True,exist_ok=True);args.report.resolve().write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2));return 0 if report['passed'] else 1
if __name__=='__main__':raise SystemExit(main())
