"""Read macOS process counters without spawning ps/lsof or writing outside the project."""
import ctypes as c
import sys
import time


class Usage(c.Structure):
    # RUSAGE_INFO_V0, SDK sys/resource.h. CPU counters are Mach absolute ticks.
    _fields_ = [('uuid', c.c_uint8 * 16)] + [(name, c.c_uint64) for name in (
        'user_time', 'system_time', 'package_wakeups', 'interrupt_wakeups',
        'pageins', 'wired_size', 'resident_size', 'phys_footprint',
        'start_abstime', 'exit_abstime')]


class Task(c.Structure):
    # PROC_PIDTASKINFO, SDK sys/proc_info.h.
    _fields_ = [(name, c.c_uint64) for name in (
        'virtual_size', 'resident_size', 'total_user', 'total_system',
        'threads_user', 'threads_system')]
    _fields_ += [(name, c.c_int32) for name in (
        'policy', 'faults', 'pageins', 'cow_faults', 'messages_sent',
        'messages_received', 'syscalls_mach', 'syscalls_unix', 'csw',
        'threadnum', 'numrunning', 'priority')]


class ProcessStats:
    def __init__(self):
        if sys.platform != 'darwin':
            raise RuntimeError('macOS process counters only')
        self.lib = c.CDLL('/usr/lib/libproc.dylib', use_errno=True)
        self.lib.proc_pid_rusage.argtypes = [c.c_int, c.c_int, c.c_void_p]
        self.lib.proc_pid_rusage.restype = c.c_int
        self.lib.proc_pidinfo.argtypes = [c.c_int, c.c_int, c.c_uint64, c.c_void_p, c.c_int]
        self.lib.proc_pidinfo.restype = c.c_int
        class Timebase(c.Structure):
            _fields_ = [('numer', c.c_uint32), ('denom', c.c_uint32)]
        system = c.CDLL('/usr/lib/libSystem.B.dylib')
        system.mach_timebase_info.argtypes = [c.POINTER(Timebase)]
        system.mach_timebase_info.restype = c.c_int
        timebase = Timebase()
        if system.mach_timebase_info(c.byref(timebase)) != 0 or not timebase.denom:
            raise RuntimeError('Mach timebase unavailable')
        self.numer, self.denom = timebase.numer, timebase.denom
        self.previous = {}

    def sample(self, pid):
        usage, task = Usage(), Task()
        if self.lib.proc_pid_rusage(pid, 0, c.byref(usage)) != 0:
            raise OSError(c.get_errno(), f'proc_pid_rusage({pid})')
        if self.lib.proc_pidinfo(pid, 4, 0, c.byref(task), c.sizeof(task)) != c.sizeof(task):
            raise OSError(c.get_errno(), f'proc_pidinfo({pid})')
        size = self.lib.proc_pidinfo(pid, 1, 0, None, 0)
        if not 0 < size <= 1_048_576:
            raise RuntimeError(f'invalid descriptor list size: {size}')
        fds = c.create_string_buffer(size)
        size = self.lib.proc_pidinfo(pid, 1, 0, fds, len(fds))
        if size < 0 or size % 8:
            raise RuntimeError(f'invalid descriptor list result: {size}')
        now = time.monotonic_ns()
        cpu = (usage.user_time + usage.system_time) * self.numer // self.denom
        previous = self.previous.get(pid)
        if previous and previous[2] != usage.start_abstime:
            raise RuntimeError(f'PID {pid} was reused')
        self.previous[pid] = now, cpu, usage.start_abstime
        return {
            'pid': pid, 'monotonic_ns': now, 'cpu_ns': cpu,
            'mach_timebase_numer': self.numer, 'mach_timebase_denom': self.denom,
            'cpu_percent_one_core': 100 * (cpu - previous[1]) / (now - previous[0]) if previous else None,
            'rss_bytes': usage.resident_size, 'physical_footprint_bytes': usage.phys_footprint,
            'threads': task.threadnum, 'file_descriptors': size // 8,
            'interrupt_wakeups': usage.interrupt_wakeups, 'pageins': usage.pageins,
        }
