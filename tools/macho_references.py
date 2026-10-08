"""Read Mach-O load-command references without otool or packaging's text parser.

Supports thin 32/64-bit and universal binaries, both byte orders. Invalid
recognized binaries fail closed; ordinary bundle resources return no slices.
"""
from pathlib import Path
import struct

THIN = {b'\xce\xfa\xed\xfe': ('<', 28), b'\xcf\xfa\xed\xfe': ('<', 32),
        b'\xfe\xed\xfa\xce': ('>', 28), b'\xfe\xed\xfa\xcf': ('>', 32)}
FAT = {b'\xca\xfe\xba\xbe': ('>', False), b'\xbe\xba\xfe\xca': ('<', False),
       b'\xca\xfe\xba\xbf': ('>', True), b'\xbf\xba\xfe\xca': ('<', True)}
DYLIB = {0xc: 'dependency', 0xd: 'install_name', 0x18: 'dependency',
         0x1f: 'dependency', 0x20: 'dependency', 0x23: 'dependency'}


def _read(handle, count):
    data = handle.read(count)
    if len(data) != count:
        raise ValueError('truncated Mach-O')
    return data


def _slice(handle, offset, size):
    handle.seek(offset)
    magic = _read(handle, 4)
    if magic not in THIN:
        raise ValueError('invalid Mach-O slice magic')
    endian, header_size = THIN[magic]
    header = magic + _read(handle, header_size - 4)
    count, length = struct.unpack_from(endian + 'II', header, 16)
    if length > 16 * 1024 * 1024 or header_size + length > size or count > length // 8:
        raise ValueError('invalid Mach-O command bounds')
    commands = _read(handle, length)
    cursor = 0
    references = []
    for _ in range(count):
        if cursor + 8 > length:
            raise ValueError('truncated Mach-O command header')
        command, command_size = struct.unpack_from(endian + 'II', commands, cursor)
        if command_size < 8 or cursor + command_size > length:
            raise ValueError('invalid Mach-O command size')
        base = command & 0x7fffffff
        kind = 'rpath' if base == 0x1c else DYLIB.get(base)
        if kind:
            minimum = 12 if kind == 'rpath' else 24
            if command_size < minimum:
                raise ValueError('truncated Mach-O string command')
            start = struct.unpack_from(endian + 'I', commands, cursor + 8)[0]
            if start < minimum or start >= command_size:
                raise ValueError('invalid Mach-O string offset')
            raw = commands[cursor + start:cursor + command_size]
            end = raw.find(b'\0')
            if end <= 0:
                raise ValueError('unterminated or empty Mach-O reference')
            references.append((kind, raw[:end].decode('utf-8', errors='strict')))
        cursor += command_size
    if cursor != length:
        raise ValueError('Mach-O command count/size mismatch')
    return references


def references(path):
    """Return references for every architecture; [] for non-Mach-O resources."""
    path = Path(path)
    size = path.stat().st_size
    with path.open('rb') as handle:
        magic = handle.read(4)
        if magic in THIN:
            return [_slice(handle, 0, size)]
        if magic not in FAT:
            return []
        endian, wide = FAT[magic]
        count, = struct.unpack(endian + 'I', _read(handle, 4))
        if not 1 <= count <= 64:
            raise ValueError('invalid Mach-O architecture count')
        entry_size = 32 if wide else 20
        entries = _read(handle, entry_size * count)
        slices = []
        for index in range(count):
            offset, length = struct.unpack_from(endian + ('QQ' if wide else 'II'),
                                                 entries, index * entry_size + 8)
            if offset < 8 + len(entries) or length < 28 or offset + length > size:
                raise ValueError('invalid Mach-O architecture bounds')
            slices.append(_slice(handle, offset, length))
        return slices


def relocation_problems(app):
    app = Path(app)
    problems = []
    for path in sorted(app.rglob('*')):
        if not path.is_file():
            continue
        try:
            slices = references(path)
        except (ValueError, UnicodeError, OSError) as error:
            problems.append(f'{path.relative_to(app)}: {error}')
            continue
        for index, refs in enumerate(slices):
            for kind, value in refs:
                if kind == 'dependency' and value.startswith(('/usr/lib/', '/System/')):
                    continue
                allowed = ('@loader_path/', '@executable_path/', '@rpath/')
                if value == '@loader_path' or value.startswith(allowed):
                    continue
                problems.append(f'{path.relative_to(app)} [slice {index}]: {kind} {value}')
    return problems
