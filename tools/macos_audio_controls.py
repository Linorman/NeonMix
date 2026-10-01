"""Small Core Audio control-thread helper for explicit-device acceptance probes."""
import ctypes as C
import math
import sys


def fourcc(value):
    return int.from_bytes(value.encode('ascii'), 'big')


class Address(C.Structure):
    _fields_ = [('selector', C.c_uint32), ('scope', C.c_uint32), ('element', C.c_uint32)]


class DeviceControls:
    def __init__(self, uid):
        if sys.platform != 'darwin':
            raise RuntimeError('Core Audio controls require macOS')
        self.ca = C.CDLL('/System/Library/Frameworks/CoreAudio.framework/CoreAudio')
        cf = C.CDLL('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation')
        self.cf = cf
        query = [C.c_uint32, C.POINTER(Address), C.c_uint32, C.c_void_p]
        self.ca.AudioObjectGetPropertyDataSize.argtypes = query + [C.POINTER(C.c_uint32)]
        self.ca.AudioObjectGetPropertyData.argtypes = query + [C.POINTER(C.c_uint32), C.c_void_p]
        self.ca.AudioObjectSetPropertyData.argtypes = query + [C.c_uint32, C.c_void_p]
        for name in ['AudioObjectGetPropertyDataSize', 'AudioObjectGetPropertyData', 'AudioObjectSetPropertyData']:
            getattr(self.ca, name).restype = C.c_int32
        cf.CFStringGetCString.argtypes = [C.c_void_p, C.c_void_p, C.c_long, C.c_uint32]
        cf.CFStringGetCString.restype = C.c_bool
        cf.CFRelease.argtypes = [C.c_void_p]
        cf.CFRelease.restype = None
        cf.CFStringCreateWithBytes.argtypes = [C.c_void_p, C.c_void_p, C.c_long, C.c_uint32, C.c_ubyte]
        cf.CFStringCreateWithBytes.restype = C.c_void_p
        address = Address(fourcc('dev#'), fourcc('glob'), 0)
        size = C.c_uint32()
        self.check(self.ca.AudioObjectGetPropertyDataSize(1, C.byref(address), 0, None, C.byref(size)))
        devices = (C.c_uint32 * (size.value // 4))()
        self.check(self.ca.AudioObjectGetPropertyData(1, C.byref(address), 0, None, C.byref(size), devices))
        for device in devices:
            value = self.read(device, 'uid ', 'glob', C.c_void_p)
            try:
                buffer = C.create_string_buffer(4096)
                if cf.CFStringGetCString(value, buffer, len(buffer), 0x08000100) and buffer.value.decode() == uid:
                    self.device = device
                    return
            finally:
                cf.CFRelease(value)
        raise RuntimeError(f'Core Audio device UID not found: {uid}')

    @staticmethod
    def check(status):
        if status:
            raise RuntimeError(f'Core Audio property operation failed: OSStatus {status}')

    def read(self, device, selector, scope, kind):
        address = Address(fourcc(selector), fourcc(scope), 0)
        value, size = kind(), C.c_uint32(C.sizeof(kind))
        self.check(self.ca.AudioObjectGetPropertyData(device, C.byref(address), 0, None, C.byref(size), C.byref(value)))
        if size.value != C.sizeof(kind):
            raise RuntimeError('Unexpected Core Audio property size')
        return value.value

    def muted(self):
        return self.read(self.device, 'mute', 'outp', C.c_uint32)

    def name(self):
        value = self.read(self.device, 'lnam', 'glob', C.c_void_p)
        try:
            buffer = C.create_string_buffer(4096)
            if not self.cf.CFStringGetCString(value, buffer, len(buffer), 0x08000100):
                raise RuntimeError('Device name is not bounded UTF-8')
            return buffer.value.decode('utf-8')
        finally:
            self.cf.CFRelease(value)

    def set_owned_name(self, name):
        data = name.encode('utf-8')
        if not data or len(data) > 256 or any(ord(c) < 32 for c in name):
            raise ValueError('Invalid owned device name')
        reference = self.cf.CFStringCreateWithBytes(None, data, len(data), 0x08000100, 0)
        if not reference:
            raise RuntimeError('Could not create owned device name')
        try:
            self.write(self.device, 'nmna', 'glob', C.c_void_p(reference))
        finally:
            self.cf.CFRelease(reference)

    def set_muted(self, muted):
        address = Address(fourcc('mute'), fourcc('outp'), 0)
        value = C.c_uint32(bool(muted))
        self.check(self.ca.AudioObjectSetPropertyData(self.device, C.byref(address), 0, None, C.sizeof(value), C.byref(value)))

    def rate(self):
        return self.read(self.device, 'nsrt', 'glob', C.c_double)

    def set_rate(self, rate):
        address = Address(fourcc('nsrt'), fourcc('glob'), 0)
        value = C.c_double(rate)
        self.check(self.ca.AudioObjectSetPropertyData(self.device, C.byref(address), 0, None, C.sizeof(value), C.byref(value)))

    def volume(self):
        return self.read(self.device, 'volm', 'outp', C.c_float)

    def volume_db(self):
        return self.read(self.device, 'vold', 'outp', C.c_float)

    def set_volume(self, volume):
        if not math.isfinite(volume) or not 0 <= volume <= 1:
            raise ValueError('volume must be finite and between 0 and 1')
        self.write(self.device, 'volm', 'outp', C.c_float(volume))

    def write(self, device, selector, scope, value):
        address = Address(fourcc(selector), fourcc(scope), 0)
        self.check(self.ca.AudioObjectSetPropertyData(device, C.byref(address), 0, None, C.sizeof(value), C.byref(value)))

    def default_outputs(self):
        return {selector: self.read(1, selector, 'glob', C.c_uint32) for selector in ('dOut', 'sOut')}

    def select_default_output(self):
        self.write(1, 'dOut', 'glob', C.c_uint32(self.device))

    def restore_default_outputs(self, values):
        for selector, device in values.items():
            self.write(1, selector, 'glob', C.c_uint32(device))
