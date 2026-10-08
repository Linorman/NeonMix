"""Packaging regressions; uses project-local fixtures and no third-party modules."""
import subprocess
import unittest
from unittest.mock import patch

from tools import package_macos as package


class RpathTests(unittest.TestCase):
    def test_rpath_load_commands_preserve_complete_paths(self):
        text = """fixture:
Load command 0
          cmd LC_LOAD_DYLIB
      cmdsize 64
         path /not/an/rpath (offset 12)
Load command 1
          cmd LC_RPATH
      cmdsize 80
         path /Volumes/My Drive/霓虹 混音/lib (offset 12)
Load command 2
          cmd LC_RPATH
      cmdsize 40
         path @loader_path/../Frameworks (offset 12)
Load command 3
          cmd LC_RPATH
      cmdsize 40
         path /plain/lib (offset 12)
"""
        with patch.object(package, 'output', return_value=text):
            self.assertEqual(package.rpaths('fixture'), [
                '/Volumes/My Drive/霓虹 混音/lib',
                '@loader_path/../Frameworks', '/plain/lib'])

    def test_delete_failure_is_fatal_and_never_adds_replacement(self):
        calls = []

        def tool(*args, **kwargs):
            calls.append(args)
            if kwargs.get('check', True):
                raise subprocess.CalledProcessError(1, args, stderr='injected error')
            return subprocess.CompletedProcess(args, 1)

        with patch.object(package, 'rpaths', return_value=['/old path/lib']), \
                patch.object(package, 'run', side_effect=tool):
            with self.assertRaises(subprocess.CalledProcessError):
                package.set_rpath('fixture', '@loader_path')
        self.assertEqual(len(calls), 1, 'must stop at first failed mutation')


class TransactionArchiveTests(unittest.TestCase):
    def test_private_journal_and_scoped_staging_never_enter_archives(self):
        from tools.archive_policy import excluded
        for name in ['profile/server.json.neonmix-transaction.json','deep/.neonmix-config-0123456789abcdef-00000000-0000-4000-8000-000000000001.tmp']:
            self.assertTrue(excluded(name))
        self.assertFalse(excluded('docs/config-transaction-guide.md'))

if __name__ == '__main__':
    unittest.main()
