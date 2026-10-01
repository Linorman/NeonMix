#!/usr/bin/env python3
"""macOS E07 native IPC/lifecycle probe. All disposable state stays in the project."""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import socket
import struct
import subprocess
import time
import uuid

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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", help="Exact physical output device ID; otherwise select the Mac built-in speaker")
    parser.add_argument("--evidence", default="docs/evidence/e07/background-macos.json")
    parser.add_argument("--release", action="store_true", help="Use the packaged target/release executables instead of target/debug")
    parser.add_argument("--client-crash", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.client_crash:
        ipc(Path(args.client_crash), "snapshot", credential="hub/admin.json", hub="https://localhost:7443")
        os._exit(77)
    if platform.system() != "Darwin":
        raise RuntimeError("This probe is intentionally macOS-only")
    directory = ROOT / ".local/tmp" / f"e07-background-{uuid.uuid4().hex[:12]}"
    directory.mkdir(mode=0o700)
    report = {"platform": "macOS", "scenarios": {}, "limits": {"ipc_bytes": LIMIT, "command_timeout_seconds": 25, "explicit_restart_after_failure": True}}
    binary_directory = ROOT / "target" / ("release" if args.release else "debug")
    report["build_profile"] = "release" if args.release else "debug"
    process = subprocess.Popen([str(DEV), str(binary_directory / "neonmix-background"), "--state-dir", str(directory)], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, cwd=ROOT)
    try:
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
        assert "private_key_ref" in profile and "private_key" not in profile
        assert "secret_ref" in admin and "token" not in admin
        report["scenarios"]["native_vault_profiles_without_plaintext_secrets"] = True
        tone = ipc(directory, "test_tone", output=output)
        report["scenarios"]["explicit_physical_output_low_level_test_tone"] = bool(tone)
        ipc(directory, "hub_start")
        snapshot = wait_for(lambda: ipc(directory, "snapshot", credential="hub/admin.json", hub="https://localhost:7443"))
        assert snapshot["viewer"]["role"] == "admin"
        assert snapshot["output"]["id"] == output
        report["scenarios"]["authenticated_viewer_role_and_real_snapshot"] = True
        pid = ipc(directory, "status")["hub"]["pid"]
        crashed = subprocess.run([str(DEV), "python3", str(Path(__file__).resolve()), "--client-crash", str(directory)], cwd=ROOT, check=False)
        assert crashed.returncode == 77
        reopened = ipc(directory, "status")
        assert reopened["hub"]["running"] and reopened["hub"]["pid"] == pid
        report["scenarios"]["ui_client_crash_does_not_stop_audio_and_reopen_is_authoritative"] = True
        invitation = ipc(directory, "invite", credential="hub/admin.json", hub="https://localhost:7443", out="invitations/probe.json", seconds=60)
        invitation_text = json.loads(invitation["invitation"])
        try:
            ipc(directory, "pair_text", invitation=invitation["invitation"], name="Self Sender", hub="https://localhost:7443")
            raise AssertionError("self-pairing was permitted")
        except RuntimeError as error:
            assert "自连接" in str(error)
        assert not list((directory / "commands").iterdir())
        ipc(directory, "cancel_invite", credential="hub/admin.json", hub="https://localhost:7443", invitation_id=invitation_text["invitation_id"])
        assert not (directory / "invitations/probe.json").exists()
        second_invitation = ipc(directory, "invite", credential="hub/admin.json", hub="https://localhost:7443", out="invitations/probe.json", seconds=60)
        second_text = json.loads(second_invitation["invitation"])
        ipc(directory, "cancel_invite", credential="hub/admin.json", hub="https://localhost:7443", invitation_id=second_text["invitation_id"])
        assert not (directory / "invitations/probe.json").exists()
        report["scenarios"]["repeat_invitation_after_cancel_uses_ephemeral_file"] = True
        report["scenarios"]["interactive_invitation_cancel_self_connection_rejected_and_temporary_cleanup"] = True
        diagnostics = ipc(directory, "diagnostics", credential="hub/admin.json", hub="https://localhost:7443")
        serialized = json.dumps(diagnostics)
        for private in [profile["certificate"], profile["private_key_ref"], profile["admin_token_ref"], invitation_text["secret"], str(directory), output]:
            assert private not in serialized
        assert "meters" in diagnostics["remote"]
        report["scenarios"]["diagnostics_redacted_with_real_meters"] = True
        exported = ipc(directory, "export_diagnostics", credential="hub/admin.json", hub="https://localhost:7443")
        export_path = directory / exported["file"]
        assert export_path.stat().st_mode & 0o777 == 0o600
        export_text = export_path.read_text()
        for private in [profile["certificate"], profile["private_key_ref"], profile["admin_token_ref"], invitation_text["secret"], str(directory), output, "E07 测试房间"]:
            assert private not in export_text
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
        wait_for(lambda: ipc(directory, "snapshot", credential="hub/admin.json", hub="https://localhost:7443"))
        ipc(directory, "hub_stop")
        assert not ipc(directory, "status")["hub"]["running"]
        report["scenarios"]["settings_revision_and_explicit_graceful_stop"] = True
        ipc(directory, "hub_start")
        wait_for(lambda: ipc(directory, "snapshot", credential="hub/admin.json", hub="https://localhost:7443"))
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
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=12)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        # Remove only references created by this disposable fixture.
        if (directory / "hub/server.json").exists():
            profile = json.loads((directory / "hub/server.json").read_text())
            for reference in [profile.get("private_key_ref"), profile.get("admin_token_ref")]:
                if reference:
                    result = subprocess.run([str(DEV), "/usr/bin/security", "delete-generic-password", "-s", "com.neonmix.identity.v1", "-a", reference], capture_output=True, cwd=ROOT)
                    if result.returncode not in (0, 44):
                        raise RuntimeError("Could not remove fixture Keychain entry")
        shutil.rmtree(directory)
    evidence = ROOT / args.evidence
    evidence.parent.mkdir(parents=True, exist_ok=True)
    evidence.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(report, ensure_ascii=False))


if __name__ == "__main__":
    main()
