#!/usr/bin/env python3
"""Measure ordinary Linux epoll timeout lateness without media/native codecs."""
import argparse
import json
import os
from pathlib import Path
import select
import sys
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--seconds', type=int, default=60)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
if sys.platform != 'linux' or not 1 <= args.seconds <= 3600:
    parser.error('requires Linux and 1..3600 seconds')
root = Path(__file__).resolve().parents[1]
output = args.output.resolve()
if not output.is_relative_to(root):
    parser.error('output must stay in this project')

origin = time.monotonic_ns()
deadline = origin + args.seconds * 1_000_000_000
requested = 1_000_000
count = 0
slow = []
max_lateness = 0
before = Path('/proc/stat').read_text().splitlines()[0]
with select.epoll() as poll:
    while time.monotonic_ns() < deadline:
        wall, cpu = time.monotonic_ns(), time.thread_time_ns()
        poll.poll(requested / 1e9)
        elapsed, used = time.monotonic_ns() - wall, time.thread_time_ns() - cpu
        late = max(0, elapsed - requested)
        count += 1
        max_lateness = max(max_lateness, late)
        if late >= 10_000_000:
            slow.append({'at_ns': wall - origin, 'wall_ns': elapsed, 'cpu_ns': used})
            slow = sorted(slow, key=lambda item: item['wall_ns'], reverse=True)[:32]
result = {
    'elapsed_ns': time.monotonic_ns() - origin,
    'policy': os.sched_getscheduler(0),
    'priority': os.sched_getparam(0).sched_priority,
    'requested_wait_ns': requested,
    'samples': count,
    'max_wait_lateness_ns': max_lateness,
    'slowest_samples': slow,
    'proc_cpu_before': before,
    'proc_cpu_after': Path('/proc/stat').read_text().splitlines()[0],
    'scope': 'diagnostic only; no audio acceptance and no scheduling changes',
}
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
