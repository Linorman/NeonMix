#!/usr/bin/env python3
"""macOS completion probes against production Sender/Hub, with real faults."""
import argparse
import concurrent.futures
import http.client
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import uuid
from lan_fault_gateway import Gateway
from macos_audio_controls import DeviceControls

ROOT = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--device', required=True)
args = p.parse_args()
if sys.platform != 'darwin':
    p.error('macOS only')
binary, audio = ROOT / 'target/release/neonmix-hub', ROOT / 'target/release/neonmix-audio'
out = ROOT / 'artifacts/completion' / time.strftime('%Y%m%d-%H%M%S')
out.mkdir(parents=True)
lab = Path(tempfile.mkdtemp(prefix='completion-', dir=ROOT / '.local/tmp'))
handles, children = [], []
gateway = None
controls = DeviceControls(args.device.removeprefix('coreaudio:'))
original_rate = controls.rate()
report = {'passed': False, 'checks': {}, 'platform': sys.platform}


def start(name, argv, block_diagnostics=False):
    log, err = (out / (name + '.jsonl')).open('w'), (out / (name + '.stderr')).open('w')
    handles.extend([log, err])
    if block_diagnostics:
        read_fd, write_fd = os.pipe()
        try:
            child = subprocess.Popen(argv, cwd=ROOT, stdout=write_fd, stderr=err)
            child.stdout = os.fdopen(read_fd, 'rb')
            handles.append(child.stdout)
            first = child.stdout.readline()
            assert first, 'Sender failed before publishing startup'
            log.write(first.decode())
            log.flush()
            # Darwin pipes can grow with the first large writer. Fill the pipe
            # explicitly using atomic <= PIPE_BUF records after startup, rather
            # than assuming that a particular duration will fill its capacity.
            os.set_blocking(write_fd, False)
            padding = (json.dumps({'event': 'diagnostic_backpressure_padding', 'padding': 'x' * 128}) + '\n').encode()
            while True:
                try:
                    assert os.write(write_fd, padding) == len(padding)
                except BlockingIOError:
                    break
            # The duplicated child stdout shares write-end file status flags.
            # Restore blocking mode so the fault is a slow reader, not a writer
            # receiving EAGAIN and exiting before the queue can recover.
            os.set_blocking(write_fd, True)
            os.set_blocking(child.stdout.fileno(), False)
        finally:
            os.close(write_fd)
    else:
        child = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=err)
    children.append(child)
    return child


def rows(name):
    values = []
    for line in (out / (name + '.jsonl')).read_text().splitlines():
        try:
            values.append(json.loads(line))
        except ValueError:
            pass
    return values


def api(path='/v1/hub', body=None):
    conn = http.client.HTTPSConnection('localhost', port, context=context, timeout=10)
    try:
        headers = {'Authorization': 'Bearer ' + credential['token']}
        if body is not None:
            headers['Content-Type'] = 'application/json'
        conn.request('GET' if body is None else 'POST', path, None if body is None else json.dumps(body), headers)
        response = conn.getresponse()
        return response.status, json.loads(response.read())
    finally:
        conn.close()


def state():
    status, value = api()
    assert status == 200
    return value


def command(operation, revision=None):
    return api('/v1/commands', {'request_id': str(uuid.uuid4()), 'expected_revision': state()['revision'] if revision is None else revision, 'operation': operation})


def wait(predicate, description, timeout=10):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        try:
            value = predicate()
            if value:
                return value
        except (OSError, ValueError, KeyError, IndexError):
            pass
        time.sleep(.1)
    raise AssertionError(description)


def sender_stats(name):
    return [r for r in rows(name) if r['event'] == 'sender_stats']


try:
    controls.set_rate(48000)
    subprocess.run([str(binary), 'init', '--directory', str(lab), '--output', args.device], check=True, stdout=subprocess.DEVNULL)
    config = json.loads((lab / 'server.json').read_text())
    credential = json.loads((lab / 'admin.json').read_text())
    context = ssl.create_default_context(cadata=credential['certificate'])
    with socket.socket() as free:
        free.bind(('127.0.0.1', 0))
        port = free.getsockname()[1]
    hub = start('hub', [str(binary), 'serve', '--config', str(lab / 'server.json'), '--listen', f'127.0.0.1:{port}'])
    wait(state, 'Hub startup')
    gateway = Gateway(port, config, lab)
    watch = start('watch', [str(binary), 'watch', '--credential', str(lab / 'admin.json'), '--hub', f'https://localhost:{gateway.port}', '--seconds', '180'])
    a = start('a', [str(binary), 'send', '--credential', str(lab / 'sender-a.json'), '--hub', f'https://localhost:{gateway.port}', '--seconds', '180', '--frequency', '437'])
    b = start('b', [str(binary), 'send', '--credential', str(lab / 'sender-b.json'), '--hub', f'https://localhost:{port}', '--seconds', '180', '--frequency', '659'], block_diagnostics=True)
    wait(lambda: len(state()['streams']) == 2, 'two streams started')
    wait(lambda: gateway.bridges and sender_stats('a')[-1]['control_subscriptions'] >= 1, 'WSS and UDP bridge ready')
    bridge = gateway.bridges[0]
    time.sleep(2)
    before_diagnostic_pressure = api('/v1/diagnostics')[1]
    # A's real encrypted packets are dropped; actual protected feedback returns.
    bridge.loss_every = 2
    paused = wait(lambda: next((r for r in reversed(sender_stats('a')) if r['policy']['paused']), None), 'real congestion must pause content', timeout=25)
    assert paused['encoder_bitrate'] == 64000 and paused['encoder_dtx']
    pause_stats = api('/v1/diagnostics')[1]
    report['checks']['real_congestion_pause'] = {'sender': paused, 'hub': pause_stats, 'dropped_packets': bridge.dropped}
    bridge.loss_every = 0
    recovered = wait(lambda: next((r for r in reversed(sender_stats('a')) if r['sent_packets'] > paused['sent_packets'] and not r['policy']['paused'] and r['encoder_bitrate'] > 64000), None), 'good authenticated feedback must resume content', timeout=12)
    assert recovered['feedback_reports'] > paused['feedback_reports']
    report['checks']['real_congestion_recovery'] = recovered
    # A's actual connected UDP peer floods the Hub while B stays healthy.
    # Readiness/native callbacks must not bypass raw-datagram cooldown or
    # cause B's hardware-clock-driven lane to underrun.
    flood_before = api('/v1/diagnostics')[1]
    b_stream_for_flood = next(r for r in rows('b') if r['event'] == 'sender_started')['stream_id']
    b_lane_for_flood = flood_before['lane_stream_ids'].index(b_stream_for_flood)
    bridge.flood = True
    time.sleep(2)
    flood_after = api('/v1/diagnostics')[1]
    bridge.flood = False
    a_stream_for_flood = next(r for r in rows('a') if r['event'] == 'sender_started')['stream_id']
    a_lane_for_flood = flood_after['lane_stream_ids'].index(a_stream_for_flood)
    report['flood_observation'] = {'injected_datagrams': bridge.injected_datagrams, 'before': flood_before, 'during': flood_after}
    assert bridge.injected_datagrams > 1000, 'flood was not injected'
    assert flood_after['media_workers'][a_lane_for_flood]['receive_throttles'] > flood_before['media_workers'][a_lane_for_flood]['receive_throttles'], 'socket-empty gaps bypassed raw receive throttle'
    assert flood_after['packet_budget_drops'][a_lane_for_flood] > flood_before['packet_budget_drops'][a_lane_for_flood], 'native ingress budget was not exercised'
    assert flood_after['underrun_frames_by_lane'][b_lane_for_flood] == flood_before['underrun_frames_by_lane'][b_lane_for_flood], 'A flood caused a B underrun'
    assert flood_after['receivers'][b_lane_for_flood]['pcm_frames'] - flood_before['receivers'][b_lane_for_flood]['pcm_frames'] >= 48000
    assert flood_after['output_stats']['errors'] == 0
    report['checks']['raw_datagram_flood_isolation'] = {'injected_datagrams': bridge.injected_datagrams, 'before': flood_before, 'during': flood_after}
    wait(lambda: api('/v1/diagnostics')[1]['receivers'][a_lane_for_flood]['packets']['received'] > flood_after['receivers'][a_lane_for_flood]['packets']['received'] + 20, 'A resumes authenticated audio after flood', timeout=4)
    # Force a control disconnect and a snapshot request slower than its timeout.
    initial = sender_stats('a')[-1]
    gateway.snapshot_delay = 6
    gateway.disconnect_events()
    time.sleep(6)
    during = sender_stats('a')[-1]
    assert a.poll() is None and during['sent_packets'] - initial['sent_packets'] >= 450
    report['checks']['slow_control_does_not_stop_media'] = {'before': initial, 'during': during}
    gateway.snapshot_delay = 0
    replica = wait(lambda: next((r for r in reversed(sender_stats('a')) if r['control_subscriptions'] >= 2 and r['control_connected']), None), 'snapshot then resubscribe', timeout=15)
    assert replica['control_snapshots'] >= 2
    report['checks']['snapshot_resubscribe'] = replica
    # Block the destination rename, independently of temporary-file naming.
    old = state()
    saved_path = lab / 'state-original.json'
    state_path = lab / 'state.json'
    persisted = state_path.read_bytes()
    state_path.rename(saved_path)
    try:
        state_path.mkdir()
        status, error = command({'type': 'output_mix', 'gain_db': -6.0, 'muted': None})
        assert status == 503 and error['error'] == 'busy'
        assert state()['output'] == old['output'] and state()['revision'] == old['revision']
    finally:
        if state_path.is_dir():
            state_path.rmdir()
        saved_path.rename(state_path)
    assert state_path.read_bytes() == persisted
    report['checks']['real_persistence_failure_is_atomic'] = True
    revision = state()['revision']
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        results = list(pool.map(lambda gain: command({'type': 'output_mix', 'gain_db': gain, 'muted': None}, revision), [-12.0, -15.0]))
    assert sorted(r[0] for r in results) == [200, 409]
    assert command({'type': 'output_mix', 'gain_db': -12.0, 'muted': None})[0] == 200
    report['checks']['concurrent_revision_conflict'] = [r[0] for r in results]
    # Force the active native output to lose its format and reopen the same UID.
    before_output_fault = api('/v1/diagnostics')[1]
    b_stream = next(r for r in rows('b') if r['event'] == 'sender_started')['stream_id']
    b_lane = before_output_fault['lane_stream_ids'].index(b_stream)
    initial_b_lane = before_diagnostic_pressure['lane_stream_ids'].index(b_stream)
    assert before_output_fault['underrun_frames_by_lane'][b_lane] == before_diagnostic_pressure['underrun_frames_by_lane'][initial_b_lane], 'blocked diagnostics stopped B media'
    assert before_output_fault['receivers'][b_lane]['pcm_frames'] - before_diagnostic_pressure['receivers'][initial_b_lane]['pcm_frames'] > 48000 * 20
    report['checks']['slow_diagnostics_does_not_stop_media'] = {'before': before_diagnostic_pressure, 'during': before_output_fault}
    old_epoch = before_output_fault['output_stats']['clock_epoch']
    controls.set_rate(44100)
    wait(lambda: any(r['event'] == 'output_reopened' for r in rows('hub')), 'native output must reopen', timeout=10)
    wait(lambda: api('/v1/diagnostics')[1]['output_stats']['clock_epoch'] > old_epoch and state()['output']['available'], 'new native clock epoch', timeout=10)
    time.sleep(1)
    # Reading the selected 44.1 kHz device must not change its format again.
    cap = start('reopened-capture', [str(audio), 'capture', '--device', args.device, '--rate', '44100', '--period', '256', '--seconds', '2'])
    assert cap.wait(timeout=8) == 0
    sample = next(r for r in rows('reopened-capture') if r['event'] == 'capture_complete')
    assert sample['measurement']['peak'] > .001 and sample['stats']['errors'] == 0
    assert controls.rate() == 44100
    report['checks']['native_reopen_44100'] = {'capture': sample, 'before': before_output_fault, 'diagnostics': api('/v1/diagnostics')[1], 'old_epoch': old_epoch}
    # Output loss/reopen above intentionally disrupts both lanes. Measure A's
    # isolation from the recovered output, so its phase cannot hide or inherit
    # an underrun caused by the preceding native format fault.
    healthy_before = api('/v1/diagnostics')[1]
    # One source disappears. B must continue on the actual native output.
    bridge.blackout = True
    wait(lambda: len(state()['streams']) == 1, 'only failing stream A terminates', timeout=10)
    cap = start('healthy-b', [str(audio), 'capture', '--device', args.device, '--rate', '44100', '--period', '256', '--seconds', '3'])
    assert cap.wait(timeout=8) == 0
    sample = next(r for r in rows('healthy-b') if r['event'] == 'capture_complete')
    assert abs(sample['measurement']['estimated_frequency_hz'] - 659) < .5
    assert sample['stats']['errors'] == sample['stats']['callback_over_budget'] == 0
    assert sample['stats']['silent_frames'] < 44100 * .03
    after = api('/v1/diagnostics')[1]
    b_session=next(r for r in rows('b') if r['event']=='sender_started')
    b_lane=healthy_before['lane_stream_ids'].index(b_session['stream_id'])
    assert after['underrun_frames_by_lane'][b_lane] == healthy_before['underrun_frames_by_lane'][b_lane], 'healthy B underrun increased'
    assert b.poll() is None
    report['checks']['single_stream_fault_isolation'] = {'capture': sample, 'before': healthy_before, 'diagnostics': after}
    # Unblock the diagnostic pipe and require evidence that the bounded writer
    # queue actually filled; otherwise this was not a backpressure test.
    def drain_b_diagnostics():
        chunk = b.stdout.read(262144)
        if chunk:
            with (out / 'b.jsonl').open('ab') as log:
                log.write(chunk)
        return next((r for r in reversed(rows('b')) if r.get('diagnostic_drops', 0) > 0), None)
    report['checks']['diagnostic_backpressure_was_real'] = wait(drain_b_diagnostics, 'diagnostic writer queue must drop bounded stats', timeout=5)
    # The Watch client survives Hub restart and applies commands after resubscribe.
    last_subs = rows('watch')[-1]['subscriptions']
    hub.send_signal(signal.SIGINT)
    assert hub.wait(timeout=8) == 0
    hub = start('hub-restarted', [str(binary), 'serve', '--config', str(lab / 'server.json'), '--listen', f'127.0.0.1:{port}'])
    wait(state, 'restarted Hub')
    wait(lambda: any(r['subscriptions'] > last_subs and r['connected'] for r in rows('watch')), 'Watch restores snapshot and events', timeout=12)
    assert command({'type': 'output_mix', 'gain_db': -9.0, 'muted': None})[0] == 200
    watch_state = wait(lambda: next((r for r in reversed(rows('watch')) if r['state'] and r['state']['output']['gain_db'] == -9), None), 'Watch applies post-restart event')
    assert not state()['streams']
    report['checks']['restart_snapshot_and_new_events'] = watch_state
    report['passed'] = True
except Exception as error:
    report['error'] = repr(error)
    try:
        report['failure_diagnostics'] = api('/v1/diagnostics')[1]
    except Exception:
        pass
finally:
    # Stop Senders/captures before their Hub, and retain the gateway until their
    # authenticated Stop requests complete. Cleanup must not invent media faults.
    shutdown_order = sorted(reversed(children), key=lambda child: child.args[1] == 'serve')
    for child in shutdown_order:
        if child.poll() is None:
            child.send_signal(signal.SIGINT)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
    if gateway:
        gateway.close()
    for handle in handles:
        handle.close()
    controls.set_rate(original_rate)
    shutil.rmtree(lab)
    (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'error': report.get('error'), 'evidence': str(out)}, indent=2))
raise SystemExit(0 if report['passed'] else 1)
