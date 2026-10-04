#!/usr/bin/env python3
"""Run macOS E00-E07 probes sequentially and preserve each run's evidence.

Invoke through tools/dev. Existing fixed-path probe reports are restored after
their fresh contents have been copied into this run's project-local directory.
"""
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
DEV = ROOT / "tools/dev"
OUT = ROOT / "docs/evidence" / ("macos-e00-e07-" + time.strftime("%Y%m%d-%H%M%S"))


def main():
    if platform.system() != "Darwin":
        raise SystemExit("macOS only")
    OUT.mkdir(parents=True)
    report = {"platform": platform.platform(), "output": str(OUT), "checks": {}}
    initial = subprocess.run(["git", "status", "--short"], cwd=ROOT, capture_output=True, text=True)
    (OUT / "git-status-before.txt").write_text(initial.stdout)
    commands = [
        ("format", ["cargo", "fmt", "--all", "--", "--check"], []),
        ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"], []),
        ("doctor", ["python3", "tools/doctor.py"], []),
        ("workspace-tests", ["cargo", "test", "--workspace", "--locked"], []),
        ("release-build", ["cargo", "build", "--workspace", "--release", "--locked"], []),
        ("simulate", ["target/release/neonmix-audio", "simulate"], []),
        ("devices", ["target/release/neonmix-audio", "devices"], []),
        ("hal-abi", ["python3", "tools/macos_hal_abi.py"], []),
        ("hal-build", ["sh", "tools/build_macos_hal.sh"], []),
        ("hal-bundle-host", ["python3", "tools/macos_hal_bundle_probe.py"], []),
        ("hal-vendor-tests", ["cargo", "test", "--manifest-path", "vendor/tympan-aspl/Cargo.toml", "--locked", "--", "--test-threads=1"], []),
        ("media-runtime", ["target/release/neonmix-hub", "runtime"], []),
        ("media-dual", ["target/release/neonmix-hub", "probe", "--seconds", "5", "--streams", "2"], []),
        ("media-loss-replay", ["target/release/neonmix-hub", "probe", "--seconds", "5", "--streams", "1", "--drop-every", "17", "--replay"], []),
        ("media-reject", ["target/release/neonmix-hub", "probe", "--seconds", "2", "--streams", "1", "--wrong-fingerprint"], []),
        ("drift-injection", ["cargo", "test", "--release", "--locked", "-p", "neonmix-core", "--test", "drift_stability", "--", "--ignored", "--nocapture"], []),
    ]
    for rate in (44100, 48000, 96000):
        commands.append((f"blackhole-{rate}", ["python3", "tools/probe.py", "--release", "--device", "coreaudio:BlackHole2ch_UID", "--rate", str(rate)], ["artifacts/virtual-probe"]))
    commands.extend([
        ("capture-lifecycle", ["python3", "tools/macos_runtime_probe.py", "--device", "coreaudio:BlackHole2ch_UID"], []),
        ("media-control", ["python3", "tools/e02_e04_probe.py", "--device", "coreaudio:BlackHole2ch_UID"], []),
        ("media-faults", ["python3", "tools/e02_e04_completion_probe.py", "--device", "coreaudio:BlackHole2ch_UID"], []),
        ("discovery-pairing", ["python3", "tools/e05_probe.py"], []),
        ("hal-binding", ["python3", "tools/e06_binding_probe.py", "--provider", "neonmix"], []),
        ("hal-app-volume", ["python3", "tools/e06_blackhole_probe.py", "--provider", "neonmix", "--output", "coreaudio:BlackHole2ch_UID"], []),
        ("background", ["python3", "tools/e07_background_probe.py", "--release", "--evidence", str(OUT / "background.json")], []),
        ("mixer", ["python3", "tools/e07_mixer_probe.py"], ["docs/evidence/e07/mixer-macos.json", "artifacts/e07/ui-real-state.json"]),
        ("native-ui", ["python3", "tools/e07_native_ui_probe.py"], ["docs/evidence/e07/native-ui-macos.json", "artifacts/e07/native-ui-final.log"]),
        ("dual-soak-300s", ["python3", "tools/hub_soak_probe.py", "--device", "coreaudio:BlackHole2ch_UID", "--seconds", "300"], []),
    ])
    def save():
        (OUT / "results.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    save()
    for name, command, protected in commands:
        backup = {}
        for relative in protected:
            path = ROOT / relative
            files = list(path.rglob("*")) if path.is_dir() else [path]
            backup[relative] = {p.relative_to(ROOT): p.read_bytes() for p in files if p.is_file()}
        prior_dirs = set((ROOT / "artifacts").glob("*/*"))
        start = time.monotonic()
        print("START", name, flush=True)
        with (OUT / (name + ".log")).open("w") as log:
            completed = subprocess.run([str(DEV), *command], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        evidence = sorted(str(p.relative_to(ROOT)) for p in set((ROOT / "artifacts").glob("*/*")) - prior_dirs if p.is_dir())
        for relative, originals in backup.items():
            path = ROOT / relative
            target = OUT / name / path.name
            if path.exists():
                target.parent.mkdir(parents=True, exist_ok=True)
                if path.is_dir():
                    shutil.copytree(path, target)
                    shutil.rmtree(path)
                else:
                    shutil.copy2(path, target)
                    path.unlink()
                evidence.append(str(target.relative_to(ROOT)))
            for original, data in originals.items():
                (ROOT / original).parent.mkdir(parents=True, exist_ok=True)
                (ROOT / original).write_bytes(data)
        report["checks"][name] = {"command": ["tools/dev", *command], "exit_code": completed.returncode, "seconds": round(time.monotonic()-start, 3), "evidence": evidence}
        if name == "release-build" and completed.returncode == 0:
            report["binary_sha256"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in (ROOT / "target/release").glob("neonmix-*") if p.is_file() and p.suffix != ".d"}
            report["source_sha256"] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for directory in ("apps", "crates", "adapters", "tools") for p in (ROOT / directory).rglob("*") if p.is_file() and p.suffix in (".rs", ".py", ".toml")}
        save()
        print("END", name, completed.returncode, flush=True)
        if name == "release-build" and completed.returncode:
            break
    print(str(OUT), flush=True)


if __name__ == "__main__":
    main()
