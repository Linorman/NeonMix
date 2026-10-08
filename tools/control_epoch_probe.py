#!/usr/bin/env python3
"""Real TLS Hub GET/WSS restart barrier; private fixtures stay project-local."""
import argparse, base64, hashlib, http.client, json, os, selectors, shutil, signal, socket, ssl, struct, subprocess, tempfile, time, uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--device",required=True,help="Explicit virtual test output; no capture or default-route change")
    parser.add_argument("--output-dir",type=Path,required=True)
    args=parser.parse_args()
    out=(ROOT/args.output_dir).resolve()
    if not out.is_relative_to(ROOT):parser.error("output directory must be project-local")
    out.mkdir(parents=True,exist_ok=True)
    lab=Path(tempfile.mkdtemp(prefix="epoch-probe-",dir=ROOT/".local/tmp"))
    binary=ROOT/"target/release/neonmix-hub"
    report={"passed":False,"binary_sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),"checks":{}}
    children=[];handles=[];sockets=[]
    try:
        subprocess.run([str(binary),"init","--directory",str(lab),"--output",args.device],cwd=ROOT,check=True,stdout=subprocess.DEVNULL)
        credentials={name:json.loads((lab/f"{name}.json").read_text()) for name in ["admin","sender-a"]}
        context=ssl.create_default_context(cadata=credentials["admin"]["certificate"])
        context.minimum_version=ssl.TLSVersion.TLSv1_3
        with socket.socket() as available:available.bind(("127.0.0.1",0));port=available.getsockname()[1]
        def api(path,body=None,who="admin"):
            conn=http.client.HTTPSConnection("localhost",port,context=context,timeout=5)
            try:
                headers={"Authorization":"Bearer "+credentials[who]["token"]}
                if body is not None:headers["Content-Type"]="application/json"
                conn.request("GET" if body is None else "POST",path,None if body is None else json.dumps(body),headers)
                response=conn.getresponse();return response.status,json.loads(response.read())
            finally:conn.close()
        def start(name):
            stdout=(out/f"{name}.jsonl").open("w");stderr=(out/f"{name}.stderr").open("w");handles.extend([stdout,stderr])
            child=subprocess.Popen([str(binary),"serve","--config",str(lab/"server.json"),"--listen",f"127.0.0.1:{port}"],cwd=ROOT,stdout=stdout,stderr=stderr)
            children.append(child)
            deadline=time.monotonic()+10
            while time.monotonic()<deadline:
                if child.poll() is not None:raise RuntimeError("owned Hub exited before Ready")
                try:
                    status,state=api("/v1/hub")
                    if status==200:return child,state
                except OSError:pass
                time.sleep(.05)
            raise TimeoutError("Hub GET readiness")
        def stop(child):
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
                try:child.wait(timeout=8)
                except subprocess.TimeoutExpired:child.kill();child.wait();raise TimeoutError("owned Hub graceful stop")
            assert child.returncode==0
        def handshake(state,legacy=False):
            conn=context.wrap_socket(socket.create_connection(("127.0.0.1",port),timeout=5),server_hostname="localhost")
            sockets.append(conn)
            key=base64.b64encode(uuid.uuid4().bytes).decode()
            query=f"after={state['event_sequence']}" if legacy else f"control_version=2&runtime_epoch={state['runtime_epoch']}&after={state['event_sequence']}"
            request=f"GET /v1/events?{query} HTTP/1.1\r\nHost: localhost:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {credentials['admin']['token']}\r\n\r\n"
            conn.sendall(request.encode());header=b""
            while b"\r\n\r\n" not in header:
                part=conn.recv(1)
                if not part:raise EOFError("handshake closed")
                header+=part
            status=int(header.split(b" ")[1])
            if status==101:
                accept=base64.b64encode(hashlib.sha1((key+"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest())
                assert accept in header;return status,None,conn
            length=next((int(line.split(b":",1)[1]) for line in header.split(b"\r\n") if line.lower().startswith(b"content-length:")),0)
            body=b""
            while len(body)<length:body+=conn.recv(length-len(body))
            conn.close();return status,json.loads(body),None
        def event(conn):
            def read(count):
                data=b""
                while len(data)<count:
                    part=conn.recv(count-len(data))
                    if not part:raise EOFError("event closed")
                    data+=part
                return data
            head=read(2);assert head[0]&15==1 and not head[1]&128
            size=head[1]&127
            if size==126:size=struct.unpack("!H",read(2))[0]
            elif size==127:size=struct.unpack("!Q",read(8))[0]
            assert size<=262144
            return json.loads(read(size))
        hub,first=start("hub-before")
        watch_error=(out/"watch.stderr").open("w");handles.append(watch_error)
        watcher=subprocess.Popen([str(binary),"watch","--credential",str(lab/"admin.json"),"--hub",f"https://localhost:{port}","--seconds","30"],cwd=ROOT,stdout=subprocess.PIPE,stderr=watch_error,bufsize=0)
        children.append(watcher)
        selector=selectors.DefaultSelector();selector.register(watcher.stdout,selectors.EVENT_READ)
        watch_buffer=b"";observations=[]
        def observe(epoch,active):
            nonlocal watch_buffer
            deadline=time.monotonic()+8
            while time.monotonic()<deadline:
                if watcher.poll() is not None:raise RuntimeError("owned control client exited during epoch recovery")
                if not selector.select(.05):continue
                watch_buffer+=os.read(watcher.stdout.fileno(),262144)
                assert len(watch_buffer)<=524288
                while b"\n" in watch_buffer:
                    line,watch_buffer=watch_buffer.split(b"\n",1)
                    row=json.loads(line)
                    if row.get("event")!="control_view" or row.get("state") is None:continue
                    state=row["state"]
                    active_count=sum(value["status"] in ["buffering","playing","network_degraded","output_lost"] for value in state["sessions"].values())
                    summary={key:row[key] for key in ["connected","snapshots","subscriptions","applied_events"]}
                    summary.update({"runtime_epoch":state["runtime_epoch"],"event_sequence":state["event_sequence"],"config_revision":state["config_revision"],"session_count":len(state["sessions"]),"active_session_count":active_count})
                    observations.append(summary)
                    if row["connected"] and state["runtime_epoch"]==epoch and active_count==active:return summary
            raise TimeoutError("control client did not resnapshot and subscribe")
        assert first["control_version"]==2 and first["event_sequence"]==first["revision"]
        _,member=api("/v1/me",who="sender-a")
        _,state=api("/v1/hub")
        udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);udp.bind(("127.0.0.1",0));sockets.append(udp)
        body={"control_version":2,"runtime_epoch":state["runtime_epoch"],"credential_id":member["device_id"],"request_id":str(uuid.uuid4()),"expected_event_sequence":state["event_sequence"],
            "operation":{"type":"start","offer":{"version":1,"codec":"opus","rate":48000,"channels":2,"packet_frames":480,"payload_type":96,"ssrc":17,"stream_epoch":1,"udp_port":udp.getsockname()[1],"certificate_sha256":"a"*64}}}
        status,response=api("/v1/sessions",body,who="sender-a");assert status==200,(status,response.get("error"))
        _,state=api("/v1/hub");assert len(state["sessions"])==1 and state["config_revision"]==first["config_revision"]
        observe(state["runtime_epoch"],1)
        status,_,ws=handshake(state);assert status==101
        _,admin=api("/v1/me")
        _,airplay_initial=api("/v2/airplay")
        bootstrap={"command_id":str(uuid.uuid4()),"runtime_epoch":state["runtime_epoch"],"credential_id":admin["device_id"],"expected_revision":airplay_initial["revision"],
            "operation":{"action":"configure","receiver_count":2,"multi_receiver":True}}
        status,airplay=api("/v2/airplay",bootstrap);assert status==200,(status,airplay.get("error"))
        assert airplay["command_version"]==3 and airplay["event_sequence"]==airplay["revision"]
        receiver=airplay["receivers"][1]["receiver_id"]
        rename={"command_version":3,"command_id":str(uuid.uuid4()),"runtime_epoch":airplay["runtime_epoch"],"credential_id":admin["device_id"],
            "expected_config_revision":airplay["config_revision"],"expected_event_sequence":airplay["event_sequence"],
            "operation":{"action":"rename_receiver","receiver_id":receiver,"name":"Epoch control test"}}
        status,renamed=api("/v2/airplay",rename);assert status==200,(status,renamed.get("error"))
        assert renamed["config_revision"]==airplay["config_revision"]+1
        status,replayed=api("/v2/airplay",rename);assert status==200 and replayed==renamed
        mixed=dict(rename);mixed["command_id"]=str(uuid.uuid4());mixed["expected_revision"]=renamed["revision"]
        status,error=api("/v2/airplay",mixed);assert status==426 and error["error"]=="upgrade_required",(status,error)
        report["checks"]["airplay_v3"]={"bootstrap_legacy_then_explicit_v3":True,"config_increment_once":True,"same_response_replay":True,"mixed_conditions_rejected":True}
        configure={"control_version":2,"runtime_epoch":state["runtime_epoch"],"credential_id":admin["device_id"],"request_id":str(uuid.uuid4()),"expected_config_revision":state["config_revision"],"operation":{"type":"output_mix","gain_db":-9.}}
        status,saved=api("/v1/commands",configure);assert status==200,(status,saved.get("error"))
        receipt=saved["receipt"]
        application=saved["media_application"]
        assert application["runtime_epoch"]==state["runtime_epoch"]
        assert saved["media_pending"]==(application["applied_config_sequence"]<application["desired_config_sequence"])
        deadline=time.monotonic()+3
        while time.monotonic()<deadline:
            _,diagnostics=api("/v1/diagnostics")
            current=diagnostics["media_application"]
            if current["applied_config_sequence"]>=application["desired_config_sequence"]:break
            time.sleep(.01)
        else:raise TimeoutError("actual callback did not acknowledge saved target")
        report["checks"]["actual_callback_application"]={"desired":application["desired_config_sequence"],"applied":current["applied_config_sequence"],"runtime_matches":current["runtime_epoch"]==state["runtime_epoch"]}
        assert receipt["config_revision"]==state["config_revision"]+1
        received=event(ws)
        assert received["runtime_epoch"]==state["runtime_epoch"] and received["event_sequence"]==state["event_sequence"]+1
        assert received["config_revision"]==receipt["config_revision"]
        ws.close()
        status,replay=api("/v1/sessions",body,who="sender-a")
        assert status==200 and replay==response
        report["checks"]["same_runtime_start_replay_same_response"]=True
        _,before_restart=api("/v1/hub")
        # Real process barrier: GET completes, that exact Hub exits, a new owned
        # process restores the same disk/cert, then the frozen cursor subscribes.
        stop(hub);new_hub,after=start("hub-after")
        assert after["hub_id"]==before_restart["hub_id"] and after["runtime_epoch"]!=before_restart["runtime_epoch"]
        assert after["config_revision"]==before_restart["config_revision"] and not after["sessions"] and not after["streams"]
        _,airplay_after=api("/v2/airplay")
        assert airplay_after["command_version"]==3 and airplay_after["config_revision"]==renamed["config_revision"]
        assert airplay_after["state"]["configuration"]==renamed["state"]["configuration"]
        status,error=api("/v2/airplay",rename);assert status==409 and error["error"]=="snapshot_required",(status,error)
        report["checks"]["airplay_v3"].update({"config_and_receiver_identity_preserved_after_restart":True,"old_epoch_rejected":True})
        status,error=api("/v1/sessions",body,who="sender-a")
        assert status==400 and error["error"]=="snapshot_required",(status,error)
        _,verified=api("/v1/hub")
        assert not verified["sessions"] and not verified["streams"] and verified["config_revision"]==after["config_revision"]
        report["checks"]["old_start_replay"]={"status":status,"error":error["error"],"no_media_recreated":True}
        status,error,_=handshake(before_restart)
        assert status==400 and error["error"]=="snapshot_required",(status,error)
        report["checks"]["old_epoch_get_to_wss"]={"status":status,"error":error["error"]}
        status,error,_=handshake(after,legacy=True)
        assert error["error"]=="upgrade_required",(status,error)
        report["checks"]["legacy_subscription"]={"status":status,"error":error["error"]}
        status,_,ws=handshake(after);assert status==101;ws.close()
        observed=observe(after["runtime_epoch"],0)
        assert observed["snapshots"]>=2 and observed["subscriptions"]>=2 and observed["session_count"]==0
        report["checks"]["real_control_client_resnapshot"]={"snapshots":observed["snapshots"],"subscriptions":observed["subscriptions"],"old_session_removed":True}
        report["client_observations"]=observations
        selector.close()
        report["checks"].update({"new_epoch_subscription":101,"real_process_restart":True,"persistent_config_preserved":True,"no_old_sessions_or_streams":True,"runtime_start_left_config_unchanged":True,"v2_config_event_receipt":True})
        stop(new_hub);report["passed"]=True
    except BaseException as error:
        report["error_type"]=type(error).__name__;report["error"]=str(error)[:300];raise
    finally:
        for conn in sockets:conn.close()
        for child in children:
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
                try:child.wait(timeout=8)
                except subprocess.TimeoutExpired:child.kill();child.wait()
        for handle in handles:handle.close()
        shutil.rmtree(lab)
        report["private_fixture_removed"]=not lab.exists()
        (out/"result.json").write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps({"passed":report["passed"],"output":str(out)}))
if __name__=="__main__":main()
