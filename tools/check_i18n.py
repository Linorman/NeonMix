#!/usr/bin/env python3
"""Run the Rust Fluent AST/schema checker with project-owned build caches."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--strict", action="store_true", help="Require complete shipped resource contracts (always enabled).")
    parser.add_argument("--pseudo", action="store_true", help="Generate isolated layout-test resources after validation.")
    parser.add_argument("--report", type=Path)
    parser.add_argument("--catalog", type=Path, help="Check a fixture catalog with the same Rust parser.")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    catalog = (args.catalog or root / "crates/i18n").resolve()
    command = ["cargo", "run", "--quiet", "-p", "neonmix-i18n", "--bin", "i18n-check", "--", str(catalog)]
    # Reuse the managed dev environment when called through tools/dev(.ps1).
    # A direct invocation enters the platform's actual dev wrapper first.
    managed = (
        bool(os.environ.get("CARGO_HOME"))
        and Path(os.environ["CARGO_HOME"]).resolve() == root / ".local/cargo"
        and bool(os.environ.get("CARGO_TARGET_DIR"))
        and Path(os.environ["CARGO_TARGET_DIR"]).resolve() == root / "target"
    )
    if not managed:
        if sys.platform == "win32":
            command = ["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(root / "tools/dev.ps1"), *command]
        else:
            command = [str(root / "tools/dev"), *command]
    if args.pseudo:
        command.extend(["--pseudo", str(root / "artifacts/i18n/pseudo/en")])
    if args.strict and args.catalog is None:
        command.extend(["--ui", str(root)])
    result = subprocess.run(command, cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    report = {"passed": result.returncode == 0, "checker": "Rust fluent-syntax AST + schema + representative rendering", "catalog": str(catalog), "stdout": result.stdout, "stderr": result.stderr}
    if args.pseudo:
        report["pseudo"] = {"passed": result.returncode == 0, "path": str(root / "artifacts/i18n/pseudo/en"), "method": "Fluent AST text-node expansion (~50%); selectors and arguments preserved"}
    if args.report:
        target = (root / args.report).resolve() if not args.report.is_absolute() else args.report.resolve()
        if not target.is_relative_to(root):
            parser.error("report output must stay inside the project")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    sys.stdout.write(result.stdout)
    sys.stderr.write(result.stderr)
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
