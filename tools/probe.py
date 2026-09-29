#!/usr/bin/env python3
"""Explicit device probe. No PCM recording; all evidence goes under artifacts/."""
import argparse
import json
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--device', required=True, help='Stable ID of a virtual device, never a default alias')
parser.add_argument('--capture-device', help='Optional distinct readable UID for a paired bridge')
parser.add_argument('--mode', choices=['virtual-device', 'loopback'], default='virtual-device')
parser.add_argument('--release', action='store_true')
parser.add_argument('--binary', type=Path, help='Explicit test binary (for cross-built native runtime probes)')
parser.add_argument('--rate', type=int, choices=[44100, 48000, 96000])
parser.add_argument('--frequency', type=float, default=437.0)
parser.add_argument('--channel', choices=['both','left','right','anti-phase'], default='both')
parser.add_argument('--expect-silence', action='store_true')
args = parser.parse_args()
binary = ROOT / 'target' / ('release' if args.release else 'debug') / ('neonmix-audio.exe' if __import__('os').name == 'nt' else 'neonmix-audio')
if args.binary:
    binary = args.binary.resolve()
rate_args = ['--rate', str(args.rate)] if args.rate else []
folder = ROOT / 'artifacts' / 'virtual-probe'
folder.mkdir(parents=True, exist_ok=True)
with (folder / 'capture.jsonl').open('w') as capture_out, (folder / 'capture.stderr').open('w') as capture_err:
    capture = subprocess.Popen([str(binary), 'capture', '--device', args.capture_device or args.device, '--mode', args.mode,
                                '--seconds', '5', *rate_args], stdout=capture_out, stderr=capture_err)
    try:
        time.sleep(0.5)
        with (folder / 'output.jsonl').open('w') as output_out, (folder / 'output.stderr').open('w') as output_err:
            output = subprocess.run([str(binary), 'play', '--device', args.device, '--seconds', '3', '--gain-db=-36', '--frequency', str(args.frequency), '--channel', args.channel, *rate_args],
                                    stdout=output_out, stderr=output_err, timeout=15)
        result = capture.wait(timeout=15)
    finally:
        if capture.poll() is None:
            capture.kill()
            capture.wait()
rows = [json.loads(line) for line in (folder / 'capture.jsonl').read_text().splitlines()]
complete = next((row['stats'] for row in rows if row['event'] == 'capture_complete'), None)
measurement = next((row.get('measurement') for row in rows if row['event'] == 'capture_complete'), None)
frequency = measurement.get('estimated_frequency_hz') if measurement else None
output_rows = [json.loads(line) for line in (folder / 'output.jsonl').read_text().splitlines()]
output_complete = next((row for row in output_rows if row['event'] == 'output_complete'), None)
output_stats = output_complete['stats'] if output_complete else None
clock = output_complete.get('clock') if output_complete else None
native_clock_advances = (clock is not None and clock['clock_timestamp_ns'] > 0
                         and clock['native_stream_frames'] is not None and clock['native_stream_frames'] > 0
                         and clock['native_internal_frames'] == clock['native_stream_frames'] * 48000 // clock['sample_rate'])
frequency_matches = frequency is not None and abs(frequency - args.frequency) <= args.frequency * 0.01
signal_matches = (complete is not None and complete['frames'] > 0 and measurement is not None and
                  (complete['silent_frames'] == complete['frames'] and measurement['peak'] == 0.0
                   if args.expect_silence else frequency_matches and complete['frames'] > complete['silent_frames']))
within_budget = (complete is not None and output_stats is not None and
                 complete['callback_over_budget'] == 0 and output_stats['callback_over_budget'] == 0)
passed = (signal_matches and within_budget and native_clock_advances and result == 0 and output.returncode == 0
          and complete is not None and complete['errors'] == 0 and output_stats['errors'] == 0)
report = {'passed': passed, 'capture_exit_code': result, 'output_exit_code': output.returncode,
          'expected_signal': 'silence' if args.expect_silence else 'tone', 'signal_matches': signal_matches, 'callbacks_within_budget': within_budget,
          'scope': 'virtual-device digital bridge, not analog latency or end-to-end networking', 'stats': complete, 'measurement': measurement,
          'expected_frequency_hz': args.frequency, 'frequency_matches': frequency_matches, 'native_clock_advances': native_clock_advances, 'output_stats': output_stats, 'clock': clock}
(folder / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
raise SystemExit(0 if passed else 1)
