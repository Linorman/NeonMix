"""Project-local, checksum-verified downloads that can resume interrupted transfers."""
import hashlib
from pathlib import Path
import subprocess


def download(url: str, path: Path, expected: str | None = None, *, max_time: int = 180) -> str:
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_name(path.name + '.part')
    if path.exists():
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if expected is None or digest == expected:
            return digest
        # An older interrupted downloader may have left a partial under the final name.
        path.replace(partial)
    if partial.exists() and expected:
        digest = hashlib.sha256(partial.read_bytes()).hexdigest()
        if digest == expected:
            partial.replace(path)
            return digest
    print('Downloading', url, flush=True)
    command = ['curl', '--http1.1', '--fail', '--location', '--silent', '--show-error',
               '--retry', '3', '--retry-all-errors', '--max-time', str(max_time),
               '--continue-at', '-', '--output', str(partial), url]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode and (result.returncode == 33 or '416' in result.stderr):
        partial.unlink(missing_ok=True)
        result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f'Download failed; partial retained at {partial}: {result.stderr}')
    digest = hashlib.sha256(partial.read_bytes()).hexdigest()
    if expected and digest != expected:
        partial.unlink()
        raise RuntimeError(f'SHA256 mismatch: {path.name}; rejected data removed')
    partial.replace(path)
    return digest
