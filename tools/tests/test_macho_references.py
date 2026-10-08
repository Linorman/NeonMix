import platform
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools import macho_references as macho
from tools import package_macos as package


def image(refs, endian='<', wide=True):
    commands = b''
    for kind, value in refs:
        base = 12 if kind == 'rpath' else 24
        raw = value.encode('utf-8') + b'\0'
        size = (base + len(raw) + 7) // 8 * 8
        code = {'rpath': 0x8000001c, 'install_name': 0xd, 'dependency': 0xc}[kind]
        header = struct.pack(endian + 'III', code, size, base) + bytes(base - 12)
        commands += header + raw + bytes(size - base - len(raw))
    magic = 0xfeedfacf if wide else 0xfeedface
    header = struct.pack(endian + ('8I' if wide else '7I'),
                         magic, 0x100000c, 0, 2, len(refs), len(commands), 0,
                         *([0] if wide else []))
    return header + commands


class MachoTests(unittest.TestCase):
    def setUp(self):
        root = Path('.local/tmp')
        root.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='rpath 霓虹 ', dir=root)
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'NeonMix.app/Contents/MacOS/fixture'
        self.path.parent.mkdir(parents=True)

    def test_binary_reader_handles_endian_width_and_unicode(self):
        refs = [('rpath', '/My Drive/霓虹/lib'), ('dependency', '/usr/lib/libSystem.B.dylib'),
                ('install_name', '@rpath/fixture.dylib')]
        for endian in '<>':
            for wide in [False, True]:
                self.path.write_bytes(image(refs, endian, wide))
                self.assertEqual(macho.references(self.path), [refs])

    def test_independent_validator_catches_modifier_omission(self):
        self.path.write_bytes(image([('rpath', '/developer directory/lib'),
                                     ('install_name', '/developer directory/fixture.dylib')]))
        with patch.object(package, 'rpaths', return_value=[]), \
                patch.object(package, 'dependencies', return_value=[]):
            with self.assertRaises(SystemExit) as error:
                package.verify_relocation(self.path.parents[2])
        self.assertIn('rpath /developer directory/lib', str(error.exception))
        self.assertIn('install_name /developer directory/fixture.dylib', str(error.exception))

    def test_universal_reader_inspects_every_slice(self):
        first = image([('rpath', '@loader_path/../Frameworks')])
        second = image([('dependency', '/developer/lib.dylib')], '>')
        offset = 48
        header = struct.pack('>II', 0xcafebabe, 2)
        header += struct.pack('>5I', 0x100000c, 0, offset, len(first), 0)
        header += struct.pack('>5I', 0x1000007, 0, offset + len(first), len(second), 0)
        self.path.write_bytes(header + first + second)
        self.assertEqual(macho.references(self.path), [[('rpath', '@loader_path/../Frameworks')],
                                                      [('dependency', '/developer/lib.dylib')]])
        problems = macho.relocation_problems(self.path.parents[2])
        self.assertEqual(len(problems), 1)
        self.assertIn('slice 1', problems[0])

    def test_malformed_recognized_image_fails_closed(self):
        self.path.write_bytes(image([('rpath', '@loader_path')])[:-1])
        with self.assertRaises(ValueError):
            macho.references(self.path)
        self.assertTrue(macho.relocation_problems(self.path.parents[2]))

    @unittest.skipUnless(platform.system() == 'Darwin', 'requires native Mach-O linker')
    def test_real_macho_rewrites_space_and_unicode_path(self):
        source = Path(self.temp.name) / 'fixture.c'
        source.write_text('int main(void) { return 0; }\n')
        old = '/Developer Path/霓虹 混音/lib'
        subprocess.run(['clang', str(source), '-o', str(self.path), '-Wl,-rpath,' + old],
                       check=True, capture_output=True)
        self.assertEqual(package.rpaths(self.path), [old])
        self.assertIn(('rpath', old), macho.references(self.path)[0])
        self.assertTrue(macho.relocation_problems(self.path.parents[2]))
        package.set_rpath(self.path, '@loader_path/../Frameworks')
        self.assertEqual(package.rpaths(self.path), ['@loader_path/../Frameworks'])
        self.assertFalse(macho.relocation_problems(self.path.parents[2]))
        subprocess.run([str(self.path.resolve())], check=True, capture_output=True)


if __name__ == '__main__':
    unittest.main()
