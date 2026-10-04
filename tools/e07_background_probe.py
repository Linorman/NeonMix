#!/usr/bin/env python3
"""macOS E07 native IPC/lifecycle probe. All disposable state stays in the project."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import shlex
import signal
import socket
import struct
import subprocess
import time
import uuid

from credential_fixture import remove_owned_fixture

ROOT = Path(__file__).resolve().parents[1]
DEV = ROOT / "tools/dev"
LIMIT = 262_144


def ipc(state_directory, kind, **fields):
    payload = json.dumps({"version": 1, "request": {"type": kind, **fields}}, ensure_ascii=False).encode()
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(45)
        connection.connect(str(state_directory / "ipc.sock"))
        connection.sendall(struct.pack("!I", len(payload)) + payload)
        def receive(size):
            output = bytearray()
            while len(output) < size:
                block = connection.recv(size - len(output))
                if not block:
                    raise RuntimeError("background closed the IPC connection")
                output.extend(block)
            return output
        size = struct.unpack("!I", receive(4))[0]
        assert size <= LIMIT
        reply = json.loads(receive(size))
        if not reply["ok"]:
            raise RuntimeError(reply["error"])
        return reply["data"]


def wait_for(predicate, seconds=8):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            last = predicate()
            if last:
                return last
        except (OSError, RuntimeError):
            pass
        time.sleep(0.1)
    raise RuntimeError(f"probe condition timed out: {last}")


def prepare_isolated_binaries(directory, source, port):
    """Keep the real processes/PIDs while changing only the fixture listen address."""
    source = source.resolve()
    source.relative_to(ROOT)
    # Release binaries use @loader_path/../../.local/gstreamer/prefix/lib.
    # Preserve that relative layout inside the owned fixture, reusing only the
    # existing project SDK; /bin/sh does not preserve DYLD_* environment values.
    binaries = directory / "target/release"
    binaries.mkdir(mode=0o700, parents=True)
    sdk = ROOT / ".local/gstreamer"
    if sdk.is_dir():
        (directory / ".local").mkdir(mode=0o700)
        (directory / ".local/gstreamer").symlink_to(sdk, target_is_directory=True)
    for name in ("neonmix-background", "neonmix-audio", "neonmix-hub"):
        destination = binaries / ("neonmix-hub-real" if name == "neonmix-hub" else name)
        shutil.copy2(source / name, destination)
    hub = shlex.quote(str(binaries / "neonmix-hub-real"))
    wrapper = binaries / "neonmix-hub"
    wrapper.write_text(
        "#!/bin/sh\nset -eu\n"
        'if [ "${1:-}" = serve ]; then\n'
        f'  exec {hub} "$@" --listen 127.0.0.1:{port}\n'
        "fi\n"
        f'exec {hub} "$@"\n'
    )
    wrapper.chmod(0o700)
    return binaries


def assert_redacted(value, private_values, serialized=None):
    """Retain the broad check, but report only labels and sanitized field paths."""
    serialized = json.dumps(value) if serialized is None else serialized
    secrets = [private for _, private in private_values]

    def safe_key(key):
        return "[private-key]" if any(secret in key for secret in secrets) else key

    def matches(current, private, path="$"):
        if isinstance(current, dict):
            found = []
            for key, item in current.items():
                field = path + "." + safe_key(key)
                if private in key:
                    found.append(field + " [key]")
                found.extend(matches(item, private, field))
            return found
        if isinstance(current, list):
            return [match for index, item in enumerate(current)
                    for match in matches(item, private, f"{path}[{index}]")]
        if private in (current if isinstance(current, str) else json.dumps(current)):
            return [path]
        return []

    for label, private in private_values:
        if private in serialized:
            paths = matches(value, private) or ["$ [serialized-only match]"]
            raise AssertionError(f"diagnostic redaction: {label} at {paths}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", help="Exact physical output device ID; otherwise select the Mac built-in speaker")
    parser.add_argument("--evidence", default="docs/evidence/e07/background-macos.json")
    parser.add_argument("--release", action="store_true", help="Use the packaged target/release executables instead of target/debug")
    parser.add_argument("--listen-port", type=int, default=0, help="Isolated IPv4 loopback port; 0 chooses a free port (default)")
    parser.add_argument("--binary-directory", type=Path, help="Project-local directory of the three already built executables")
    parser.add_argument("--client-crash", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not 0 <= args.listen_port <= 65535:
        parser.error("--listen-port must be 0..65535")
    if args.client_crash:
        if args.listen_port == 0:
            parser.error("--client-crash requires the fixture listen port")
        ipc(Path(args.client_crash), "snapshot", credential="hub/admin.json", hub=f"https://localhost:{args.listen_port}")
        os._exit(77)
    if platform.system() != "Darwin":
        raise RuntimeError("This probe is intentionally macOS-only")
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", args.listen_port))
        port = listener.getsockname()[1]
    hub_url = f"https://localhost:{port}"
    evidence = (ROOT / args.evidence).resolve()
    evidence.relative_to(ROOT)
    source_binaries = args.binary_directory or ROOT / "target" / ("release" if args.release else "debug")
    source_binaries = source_binaries.resolve()
    source_binaries.relative_to(ROOT)
    for name in ("neonmix-background", "neonmix-audio", "neonmix-hub"):
        if not (source_binaries / name).is_file():
            raise RuntimeError(f"missing built executable: {name}")
    directory = ROOT / ".local/tmp" / f"e07-background-{uuid.uuid4().hex[:12]}"
    directory.mkdir(mode=0o700)
    report = {"platform": "macOS", "scenarios": {}, "limits": {"ipc_bytes": LIMIT, "command_timeout_seconds": 25, "explicit_restart_after_failure": True}}
    report["build_profile"] = "release" if args.release else "debug"
    report["listen_port"] = port
    process = None
    try:
        binary_directory = prepare_isolated_binaries(directory, source_binaries, port)
        report["binary_sha256"] = {name: hashlib.sha256((binary_directory / name).read_bytes()).hexdigest()
                                   for name in ("neonmix-background", "neonmix-audio", "neonmix-hub-real")}
        process = subprocess.Popen([str(DEV), str(binary_directory / "neonmix-background"), "--state-dir", str(directory)], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, cwd=ROOT)
        wait_for(lambda: ipc(directory, "status"))
        assert (directory / "ipc.sock").stat().st_mode & 0o777 == 0o600
        assert directory.stat().st_mode & 0o777 == 0o700
        report["scenarios"]["private_socket_and_state"] = True
        devices = ipc(directory, "devices")
        output = args.output
        if not output:
            outputs = [device for device in devices if device.get("output") and "builtinspeaker" in device["id"].lower()]
            if len(outputs) != 1:
                raise RuntimeError("Select one physical output using --output; no device fallback is permitted")
            output = outputs[0]["id"]
        settings = {"name": "E07 测试房间", "output": output}
        ipc(directory, "hub_setup", settings=settings)
        profile = json.loads((directory / "hub/server.json").read_text())
        admin = json.loads((directory / "hub/admin.json").read_text())
        assert profile["version"] == 2 and profile["credential_store"] == "file"
        assert admin["version"] == 2 and admin["credential_store"] == "file" and admin["profile_kind"] == "admin"
        assert profile["admin_token_ref"] == admin["secret_ref"]
        assert (directory / "hub/.credentials" / (profile["private_key_ref"] + ".json")).is_file()
        assert (directory / "hub/.credentials" / (admin["secret_ref"] + ".json")).is_file()
        assert "private_key_ref" in profile and "private_key" not in profile
        assert "secret_ref" in admin and "token" not in admin
        report["scenarios"]["file_profiles_reference_dedicated_secret_entries"] = True
        tone = ipc(directory, "test_tone", output=output)
        report["scenarios"]["explicit_physical_output_low_level_test_tone"] = bool(tone)
        ipc(directory, "hub_start")
        snapshot = wait_for(lambda: ipc(directory, "snapshot", credential="hub/admin.json", hub=hub_url))
        assert snapshot["viewer"]["role"] == "admin"
        assert snapshot["output"]["id"] == output
        report["scenarios"]["authenticated_viewer_role_and_real_snapshot"] = True
        pid = ipc(directory, "status")["hub"]["pid"]
        crashed = subprocess.run([str(DEV), "python3", str(Path(__file__).resolve()), "--client-crash", str(directory), "--listen-port", str(port)], cwd=ROOT, check=False)
        assert crashed.returncode == 77
        reopened = ipc(directory, "status")
        assert reopened["hub"]["running"] and reopened["hub"]["pid"] == pid
        report["scenarios"]["ui_client_crash_does_not_stop_audio_and_reopen_is_authoritative"] = True
        invitation = ipc(directory, "invite", credential="hub/admin.json", hub=hub_url, out="invitations/probe.json", seconds=60)
        invitation_text = json.loads(invitation["invitation"])
        try:
            ipc(directory, "pair_text", invitation=invitation["invitation"], name="Self Sender", hub=hub_url)
            raise AssertionError("self-pairing was permitted")
        except RuntimeError as error:
            assert "自连接" in str(error)
        assert not list((directory / "commands").iterdir())
        ipc(directory, "cancel_invite", credential="hub/admin.json", hub=hub_url, invitation_id=invitation_text["invitation_id"])
        assert not (directory / "invitations/probe.json").exists()
        second_invitation = ipc(directory, "invite", credential="hub/admin.json", hub=hub_url, out="invitations/probe.json", seconds=60)
        second_text = json.loads(second_invitation["invitation"])
        ipc(directory, "cancel_invite", credential="hub/admin.json", hub=hub_url, invitation_id=second_text["invitation_id"])
        assert not (directory / "invitations/probe.json").exists()
        report["scenarios"]["repeat_invitation_after_cancel_uses_ephemeral_file"] = True
        report["scenarios"]["interactive_invitation_cancel_self_connection_rejected_and_temporary_cleanup"] = True
        fixture_secrets = []
        secret_kinds = {"hub_tls_key", "admin_token", "member_token", "airplay_receiver_key"}
        for path in (directory / "hub/.credentials").glob("*.json"):
            entry = json.loads(path.read_text())
            kind = entry.get("kind")
            fixture_secrets.append((kind if kind in secret_kinds else "fixture_secret", entry["value"]))
        diagnostics = ipc(directory, "diagnostics", credential="hub/admin.json", hub=hub_url)
        private_values = fixture_secrets + [
            ("certificate", profile["certificate"]),
            ("private_key_ref", profile["private_key_ref"]),
            ("admin_token_ref", profile["admin_token_ref"]),
            ("invitation_secret", invitation_text["secret"]),
            ("fixture_directory", str(directory)),
            ("output_id", output),
        ]
        assert_redacted(diagnostics, private_values)
        assert "meters" in diagnostics["remote"]
        report["scenarios"]["diagnostics_redacted_with_real_meters"] = True
        exported = ipc(directory, "export_diagnostics", credential="hub/admin.json", hub=hub_url)
        export_path = directory / exported["file"]
        assert export_path.stat().st_mode & 0o777 == 0o600
        export_text = export_path.read_text()
        assert_redacted(json.loads(export_text), private_values + [("room_name", "E07 测试房间")], export_text)
        assert "meters" in json.loads(export_text)["remote"]
        report["scenarios"]["strict_numeric_whitelist_private_diagnostic_export"] = True
        os.kill(pid, signal.SIGKILL)
        failed = wait_for(lambda: (state if not state["hub"]["running"] else None) if (state := ipc(directory, "status")) else None)
        assert failed["hub"]["error"]
        time.sleep(0.2)
        assert not ipc(directory, "status")["hub"]["running"]
        report["scenarios"]["media_process_crash_reported_without_unbounded_restart"] = True
        ipc(directory, "hub_settings", settings={"name": "E07 重命名", "output": output})
        ipc(directory, "hub_start")
        wait_for(lambda: ipc(directory, "snapshot", credential="hub/admin.json", hub=hub_url))
        ipc(directory, "hub_stop")
        assert not ipc(directory, "status")["hub"]["running"]
        report["scenarios"]["settings_revision_and_explicit_graceful_stop"] = True
        ipc(directory, "hub_start")
        wait_for(lambda: ipc(directory, "snapshot", credential="hub/admin.json", hub=hub_url))
        final_pid = ipc(directory, "status")["hub"]["pid"]
        ipc(directory, "shutdown")
        assert process.wait(timeout=10) == 0
        assert not (directory / "ipc.sock").exists()
        try:
            os.kill(final_pid, 0)
            raise AssertionError("Hub survived explicit background shutdown")
        except ProcessLookupError:
            pass
        report["scenarios"]["explicit_background_shutdown_ends_audio_process"] = True
        report["passed"] = True
    except Exception as error:
        report["passed"] = False
        report["failure_type"] = type(error).__name__
        if isinstance(error, AssertionError) and str(error).startswith("diagnostic redaction:"):
            report["redaction_failure"] = str(error)
        raise
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=12)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        remove_owned_fixture(directory)
        report["fixture_removed"] = not directory.exists()
        evidence.parent.mkdir(parents=True, exist_ok=True)
        evidence.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(report, ensure_ascii=False))


if __name__ == "__main__":
    main()
