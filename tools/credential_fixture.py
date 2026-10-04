"""Cleanup for invocation-owned fixtures; never inspect ambient credentials."""
from pathlib import Path
import hashlib
import json
import shutil
import uuid

ROOT = Path(__file__).resolve().parents[1]


def remove_owned_fixture(directory):
    directory = Path(directory)
    relative = directory.resolve().relative_to((ROOT / '.local/tmp').resolve())
    if not relative.parts or directory.is_symlink():
        raise RuntimeError('refusing unowned fixture cleanup')
    shutil.rmtree(directory)


def receiver_snapshot(receiver):
    """Read only a caller-created fixture and return metadata plus a key digest."""
    receiver = Path(receiver)
    receiver.resolve().relative_to((ROOT / '.local/tmp').resolve())
    metadata = json.loads(receiver.read_text())
    assert metadata['credential_store'] == 'file'
    if metadata['version'] == 2:
        # Preserve the legacy probe contract while checking the current durable
        # first-entry identity and its exact scoped/global trust projection.
        endpoint = metadata['receivers'][0]
        bindings = {binding['source_id']: binding for binding in metadata['bindings']
                    if binding['receiver_id'] == endpoint['receiver_id']}
        known, blocked = [], []
        for source in metadata['sources']:
            binding = bindings.get(source['source_id'])
            if source['blocked'] or source['revoked'] or (binding and binding['revoked']):
                blocked.append(source['public_key'])
            elif binding:
                known.append(source['public_key'])
        metadata = dict(version=1, credential_store='file', key_reference=endpoint['key_reference'],
                        known_keys=sorted(known), blocked_keys=sorted(blocked),
                        playback_allowed=metadata['playback_allowed'],
                        playback_mode=endpoint['default_playback_mode'])
    else:
        assert metadata['version'] == 1, 'unsupported fixture receiver profile'
    reference = metadata['key_reference']
    assert str(uuid.UUID(reference)) == reference, 'fixture key reference is not canonical'
    key = receiver.parent / '.credentials' / (reference + '.json')
    key.resolve().relative_to((ROOT / '.local/tmp').resolve())
    key.resolve().relative_to((receiver.parent / '.credentials').resolve())
    entry = json.loads(key.read_text())
    assert entry['version'] == 1 and entry['kind'] == 'airplay_receiver_key'
    return metadata, hashlib.sha256(key.read_bytes()).digest()
