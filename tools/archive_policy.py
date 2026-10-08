"""Explicit exclusions shared by build archives and their regression probe."""
from pathlib import Path


def excluded(path):
    """Exclude secret stores, worker PEMs and migration/test state at any depth."""
    parts = Path(path).parts
    return any(
        part == '.credentials'
        or part.endswith('.pending')
        or part.startswith('runtime-key-')
        or part.endswith('.neonmix-transaction.json')
        or part.startswith('.neonmix-config-')
        or part.startswith('credential-store-')
        or part.startswith('credential-migration-')
        or part.startswith('.neonmix-migration-')
        or part.startswith('staging-')
        or part.endswith('.staging')
        or part in {'staging', '.local', 'target'}
        for part in parts
    )


def copy_ignore(directory, names):
    return [name for name in names if excluded(name) or (Path(directory) / name).is_symlink()]


def assert_clean(directory):
    """Fail before publication if stale staging output contains secret paths."""
    for path in Path(directory).rglob('*'):
        if excluded(path.relative_to(directory)) or path.is_symlink():
            raise RuntimeError('archive includes excluded credential or runtime data')
