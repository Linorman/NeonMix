#!/usr/bin/env python3
"""Detect Linux distributions and switch common package indexes.

Examples:
  python3 tools/mirror_switch.py detect
  tools/dev python3 tools/mirror_switch.py set ustc --for pip,uv,cargo
  sudo python3 tools/mirror_switch.py set tsinghua --for system
  sudo python3 tools/mirror_switch.py set aliyun --for all --include-security
  python3 tools/mirror_switch.py set official --for all
  python3 tools/mirror_switch.py restore --for all

System repository support covers Ubuntu/Debian APT, Fedora 44+ DNF5, and Arch
Linux pacman. Other Linux distributions are detected but left untouched.
"""

from __future__ import annotations

import argparse
import difflib
import os
import platform
import pwd
import re
import shlex
import shutil
import stat
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable
from urllib.parse import urlsplit


MIRRORS = {
    "official": {"label": "官方", "host": None},
    "ustc": {"label": "USTC 中科大", "host": "mirrors.ustc.edu.cn"},
    "tsinghua": {
        "label": "清华 TUNA",
        "host": "mirrors.tuna.tsinghua.edu.cn",
    },
    "aliyun": {"label": "阿里云", "host": "mirrors.aliyun.com"},
}
MIRROR_ALIASES = {
    "official": "official",
    "default": "official",
    "ustc": "ustc",
    "tuna": "tsinghua",
    "tsinghua": "tsinghua",
    "清华": "tsinghua",
    "aliyun": "aliyun",
    "ali": "aliyun",
    "阿里": "aliyun",
}
TARGETS = ("system", "pip", "uv", "cargo")
BACKUP_SUFFIX = ".neonmix.bak"
ABSENT_SUFFIX = ".neonmix.absent"
UV_BEGIN = "# BEGIN NeonMix mirror-switcher"
UV_END = "# END NeonMix mirror-switcher"
DNF_BEGIN = "# BEGIN NeonMix mirror-switcher"
DNF_END = "# END NeonMix mirror-switcher"
CARGO_BEGIN = "# BEGIN NeonMix mirror-switcher"
CARGO_END = "# END NeonMix mirror-switcher"
CARGO_KEY_MARKER = "# NeonMix mirror-switcher"
CARGO_SOURCE = "neonmix-mirror"
ARCH_BEGIN = "# BEGIN NeonMix mirror-switcher"
ARCH_END = "# END NeonMix mirror-switcher"

PYPI_INDEXES = {
    "official": "https://pypi.org/simple",
    "ustc": "https://mirrors.ustc.edu.cn/pypi/simple",
    "tsinghua": "https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple",
    "aliyun": "https://mirrors.aliyun.com/pypi/simple/",
}
CARGO_INDEXES = {
    "ustc": "sparse+https://mirrors.ustc.edu.cn/crates.io-index/",
    "tsinghua": "sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/",
}


@dataclass(frozen=True)
class Distro:
    os_id: str
    id_like: tuple[str, ...]
    name: str
    version: str
    codename: str
    package_manager: str
    apt_family: str | None


@dataclass
class Change:
    path: Path
    before: str | None
    after: str | None
    mode: int | None = None
    uid: int | None = None
    gid: int | None = None


class MirrorError(Exception):
    pass


def _current_user() -> pwd.struct_passwd:
    if os.geteuid() == 0 and os.environ.get("SUDO_USER"):
        try:
            return pwd.getpwnam(os.environ["SUDO_USER"])
        except KeyError:
            pass
    return pwd.getpwuid(os.getuid())


def _read_os_release() -> dict[str, str]:
    release = Path("/etc/os-release")
    if not release.is_file():
        return {}
    values: dict[str, str] = {}
    for line in release.read_text(encoding="utf-8", errors="replace").splitlines():
        if not line or line.lstrip().startswith("#") or "=" not in line:
            continue
        key, raw = line.split("=", 1)
        try:
            parsed = shlex.split(raw, posix=True)
            values[key] = parsed[0] if parsed else ""
        except ValueError:
            values[key] = raw.strip().strip("\"'")
    return values


def detect_distro() -> Distro:
    release = _read_os_release()
    os_id = release.get("ID", "unknown").lower()
    id_like = tuple(x.lower() for x in release.get("ID_LIKE", "").split())
    candidates = {os_id, *id_like}

    if shutil.which("apt-get"):
        package_manager = "apt"
    elif shutil.which("dnf"):
        package_manager = "dnf"
    elif shutil.which("yum"):
        package_manager = "yum"
    elif shutil.which("pacman"):
        package_manager = "pacman"
    elif shutil.which("zypper"):
        package_manager = "zypper"
    elif shutil.which("apk"):
        package_manager = "apk"
    else:
        package_manager = "unknown"

    apt_family = None
    if os_id == "ubuntu" or "ubuntu" in candidates:
        apt_family = "ubuntu"
    elif os_id == "debian" or "debian" in candidates:
        apt_family = "debian"

    name = release.get("PRETTY_NAME") or release.get("NAME") or platform.system()
    return Distro(
        os_id=os_id,
        id_like=id_like,
        name=name,
        version=release.get("VERSION_ID", "unknown"),
        codename=release.get("VERSION_CODENAME", release.get("UBUNTU_CODENAME", "")),
        package_manager=package_manager,
        apt_family=apt_family,
    )


def normalize_mirror(value: str) -> str:
    key = MIRROR_ALIASES.get(value.strip().lower(), MIRROR_ALIASES.get(value.strip()))
    if key is None:
        choices = ", ".join(("official", "ustc", "tsinghua", "aliyun"))
        raise MirrorError(f"未知镜像源 {value!r}；可选值：{choices}")
    return key


def parse_targets(value: str) -> tuple[str, ...]:
    if value.strip().lower() == "all":
        return TARGETS
    selected = tuple(dict.fromkeys(part.strip().lower() for part in value.split(",") if part.strip()))
    invalid = [item for item in selected if item not in TARGETS]
    if invalid or not selected:
        raise MirrorError(f"无效的 --for 值：{', '.join(invalid) or value!r}；可选值：all,{','.join(TARGETS)}")
    return selected


def _security_suite(suites: Iterable[str]) -> bool:
    for suite in suites:
        normalized = suite.strip().lower().rstrip("/")
        if normalized.endswith("-security") or normalized.endswith("/updates"):
            return True
    return False


def native_system_kind(distro: Distro) -> str | None:
    if distro.package_manager == "apt" and distro.apt_family:
        return "apt"
    if distro.os_id == "fedora" and distro.package_manager == "dnf":
        try:
            major = int(distro.version.split(".", 1)[0])
        except ValueError:
            major = 0
        if major >= 44:
            return "fedora-dnf5"
    if distro.os_id == "arch" and distro.package_manager == "pacman":
        return "arch"
    return None


def _known_provider(host: str) -> str | None:
    for key, details in MIRRORS.items():
        if details["host"] == host:
            return key
    return None


def apt_mirror_url(
    url: str,
    family: str,
    mirror: str,
    security_suite: bool,
    include_security: bool,
) -> str:
    try:
        parsed = urlsplit(url)
        host = (parsed.hostname or "").lower().rstrip(".")
        path = parsed.path.rstrip("/")
    except ValueError:
        return url
    if parsed.scheme not in {"http", "https"} or parsed.username or parsed.password or parsed.port:
        return url

    provider = _known_provider(host)
    section: str | None = None
    is_security = security_suite

    if family == "ubuntu":
        if path == "/ubuntu-ports" and (host == "ports.ubuntu.com" or provider):
            section = "ubuntu-ports"
        elif path == "/ubuntu" and (
            host == "archive.ubuntu.com"
            or host.endswith(".archive.ubuntu.com")
            or host == "security.ubuntu.com"
            or provider
        ):
            section = "ubuntu"
            is_security = is_security or host == "security.ubuntu.com"
        if section is None:
            return url
        if is_security and not include_security and provider is None:
            return url
        if mirror == "official":
            if section == "ubuntu-ports":
                base = "https://ports.ubuntu.com/ubuntu-ports"
            elif is_security:
                base = "https://security.ubuntu.com/ubuntu"
            else:
                base = "https://archive.ubuntu.com/ubuntu"
        else:
            base = f"https://{MIRRORS[mirror]['host']}/{section}"
    elif family == "debian":
        if path in {"/debian", "/debian-security"} and (
            host.endswith(".debian.org")
            or provider
        ):
            is_security = is_security or path == "/debian-security" or host in {
                "security.debian.org",
                "security-cdn.debian.org",
            }
            section = "debian-security" if is_security else "debian"
        else:
            return url
        if is_security and not include_security and provider is None:
            return url
        if mirror == "official":
            if is_security:
                base = "https://security.debian.org/debian-security"
            else:
                base = "https://deb.debian.org/debian"
        else:
            base = f"https://{MIRRORS[mirror]['host']}/{section}"
    else:
        return url

    if parsed.path.endswith("/"):
        base += "/"
    return base


def _replace_urls(value: str, family: str, mirror: str, security: bool, include_security: bool) -> str:
    url_pattern = re.compile(r"https?://[^\s#]+")
    return url_pattern.sub(
        lambda match: apt_mirror_url(match.group(0), family, mirror, security, include_security),
        value,
    )


def transform_apt_list(text: str, family: str, mirror: str, include_security: bool) -> str:
    output: list[str] = []
    source_line = re.compile(
        r"^(?P<prefix>\s*deb(?:-src)?\s+(?:\[[^\]]+\]\s+)?)(?P<url>\S+)(?P<rest>\s+)(?P<suite>\S+)(?P<tail>.*)$"
    )
    for line in text.splitlines(keepends=True):
        match = source_line.match(line.rstrip("\r\n"))
        if not match or line.lstrip().startswith("#"):
            output.append(line)
            continue
        suite = match.group("suite")
        security = _security_suite((suite,))
        url = apt_mirror_url(match.group("url"), family, mirror, security, include_security)
        ending = "\r\n" if line.endswith("\r\n") else "\n" if line.endswith("\n") else ""
        output.append(
            f"{match.group('prefix')}{url}{match.group('rest')}{suite}{match.group('tail')}{ending}"
        )
    return "".join(output)


def transform_apt_sources(text: str, family: str, mirror: str, include_security: bool) -> str:
    lines = text.splitlines(keepends=True)
    output: list[str] = []
    block: list[str] = []

    def flush(current: list[str]) -> None:
        if not current:
            return
        suites: list[str] = []
        enabled = True
        uri_fields: set[int] = set()
        continuation_field: str | None = None
        for index, raw in enumerate(current):
            stripped = raw.strip()
            if not stripped or stripped.startswith("#"):
                continue
            if raw[:1].isspace():
                if continuation_field == "suites":
                    suites.extend(stripped.split())
                if continuation_field == "uris":
                    uri_fields.add(index)
                continue
            continuation_field = None
            if ":" not in raw:
                continue
            key, value = raw.split(":", 1)
            key = key.strip().lower()
            values = value.split("#", 1)[0].split()
            if key == "enabled" and values and values[0].lower() == "no":
                enabled = False
            if key == "suites":
                suites.extend(values)
                continuation_field = "suites"
            elif key == "uris":
                uri_fields.add(index)
                continuation_field = "uris"

        security = _security_suite(suites)
        if enabled:
            for index in uri_fields:
                line = current[index]
                match = re.match(r"^(\s*URIs\s*:\s*|\s+)(.*?)(\r?\n)?$", line, re.IGNORECASE)
                if not match:
                    continue
                body = _replace_urls(
                    match.group(2), family, mirror, security, include_security
                )
                newline = match.group(3) or ""
                current[index] = f"{match.group(1)}{body}{newline}"
        output.extend(current)

    for line in lines:
        if line.strip() == "":
            flush(block)
            block = []
            output.append(line)
        else:
            block.append(line)
    flush(block)
    return "".join(output)


def apt_changes(distro: Distro, mirror: str, include_security: bool) -> list[Change]:
    if native_system_kind(distro) != "apt" or distro.apt_family is None:
        raise MirrorError(f"{distro.name} 不支持当前 APT 源处理器。")
    root = Path("/etc/apt")
    files: list[Path] = [root / "sources.list"]
    source_dir = root / "sources.list.d"
    if source_dir.is_dir():
        files.extend(sorted(source_dir.glob("*.list")))
        files.extend(sorted(source_dir.glob("*.sources")))

    changes: list[Change] = []
    for path in dict.fromkeys(files):
        if not path.exists() or path.is_symlink() or not path.is_file():
            continue
        before = path.read_text(encoding="utf-8", errors="replace")
        if path.suffix == ".sources":
            after = transform_apt_sources(before, distro.apt_family, mirror, include_security)
        else:
            after = transform_apt_list(before, distro.apt_family, mirror, include_security)
        if after != before:
            changes.append(Change(path, before, after, stat.S_IMODE(path.stat().st_mode)))
    return changes


def fedora_changes(mirror: str) -> list[Change]:
    path = Path("/etc/dnf/repos.override.d/99-neonmix-mirror.repo")
    if path.is_symlink():
        raise MirrorError(f"拒绝覆盖符号链接：{path}")
    before = path.read_text(encoding="utf-8") if path.exists() else None
    if mirror == "official":
        if before is None:
            return []
        if DNF_BEGIN not in before or DNF_END not in before:
            raise MirrorError(f"{path} 已存在且不是本脚本创建的文件，未删除。")
        after = _managed_block(before, DNF_BEGIN, DNF_END, "", official=True)
        return [
            Change(path, before, after or None, stat.S_IMODE(path.stat().st_mode))
        ] if before != after else []

    if before is not None and (DNF_BEGIN not in before or DNF_END not in before):
        raise MirrorError(f"{path} 已存在且不是本脚本创建的文件，未覆盖。")

    host = MIRRORS[mirror]["host"]
    body = (
        "[fedora]\n"
        f"baseurl=https://{host}/fedora/releases/$releasever/Everything/$basearch/os/\n"
        "metalink=\n\n"
        "[updates]\n"
        f"baseurl=https://{host}/fedora/updates/$releasever/Everything/$basearch/\n"
        "metalink="
    )
    after = _managed_block(before or "", DNF_BEGIN, DNF_END, body, official=False)
    return [Change(path, before, after, stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o644)] if before != after else []


def arch_changes(mirror: str) -> list[Change]:
    path = Path("/etc/pacman.d/mirrorlist")
    if path.is_symlink():
        raise MirrorError(f"拒绝覆盖符号链接：{path}")
    before = path.read_text(encoding="utf-8") if path.exists() else None
    if mirror == "official":
        server = "https://geo.mirror.pkgbuild.com/$repo/os/$arch"
    else:
        server = f"https://{MIRRORS[mirror]['host']}/archlinux/$repo/os/$arch"
    block = f"{ARCH_BEGIN}\nServer = {server}\n{ARCH_END}"
    text = before or ""
    if text.count(ARCH_BEGIN) != text.count(ARCH_END) or text.count(ARCH_BEGIN) > 1:
        raise MirrorError(f"{path} 中的 NeonMix 管理标记不完整或重复。")
    if ARCH_BEGIN in text:
        start = text.index(ARCH_BEGIN)
        end = text.index(ARCH_END, start) + len(ARCH_END)
        after = text[:start] + block + text[end:]
        after = after.rstrip("\n") + ("\n" if after else "")
    else:
        after = f"{block}\n\n{text}" if text else f"{block}\n"
    return [Change(path, before, after, stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o644)] if before != after else []


def build_system_changes(distro: Distro, mirror: str, include_security: bool) -> list[Change]:
    kind = native_system_kind(distro)
    if kind == "apt":
        return apt_changes(distro, mirror, include_security)
    if kind == "fedora-dnf5":
        if include_security:
            raise MirrorError("--include-security 仅用于 Debian/Ubuntu APT 源。")
        return fedora_changes(mirror)
    if kind == "arch":
        if include_security:
            raise MirrorError("--include-security 仅用于 Debian/Ubuntu APT 源。")
        return arch_changes(mirror)
    raise MirrorError(
        f"检测到 {distro.name}（包管理器：{distro.package_manager}）；"
        "当前 system 源切换支持 Ubuntu/Debian APT、Fedora 44+ DNF5 和 Arch Linux。"
    )


def _set_pip_index(text: str, index_url: str) -> str:
    lines = text.splitlines()
    section_ranges: list[tuple[int, int, str]] = []
    section = ""
    section_start = 0
    for index, line in enumerate(lines):
        header = re.match(r"^\s*\[([^\]]+)\]\s*(?:[#;].*)?$", line)
        if header:
            if section:
                section_ranges.append((section_start, index, section.lower()))
            section = header.group(1).strip()
            section_start = index + 1
    if section:
        section_ranges.append((section_start, len(lines), section.lower()))

    globals_found = [(start, end) for start, end, name in section_ranges if name == "global"]
    if not globals_found:
        if lines and lines[-1].strip():
            lines.append("")
        lines.extend(("[global]", f"index-url = {index_url}"))
    else:
        start, end = globals_found[0]
        matching = [
            index
            for index in range(start, end)
            if re.match(r"^\s*index-url\s*=", lines[index], re.IGNORECASE)
        ]
        if matching:
            first = matching[0]
            indent = re.match(r"^\s*", lines[first]).group(0)
            lines[first] = f"{indent}index-url = {index_url}"
            for duplicate in reversed(matching[1:]):
                del lines[duplicate]
        else:
            lines.insert(end, f"index-url = {index_url}")

    result = "\n".join(lines)
    return result + ("\n" if result and not result.endswith("\n") else "")


def _managed_block(text: str, begin: str, end: str, body: str, official: bool) -> str:
    begin_count = text.count(begin)
    end_count = text.count(end)
    if begin_count != end_count or begin_count > 1:
        raise MirrorError(f"配置中的管理标记不完整或重复：{begin}")
    replacement = "" if official else f"{begin}\n{body.rstrip()}\n{end}"
    if begin_count:
        start = text.index(begin)
        finish = text.index(end, start) + len(end)
        result = text[:start] + replacement + text[finish:]
    elif official:
        return text
    else:
        prefix = text.rstrip()
        result = f"{prefix}\n\n{replacement}\n" if prefix else f"{replacement}\n"
    return result.strip("\n") + ("\n" if result.strip("\n") else "")


def uv_config_change(path: Path, mirror: str) -> Change:
    if path.is_symlink():
        raise MirrorError(f"拒绝覆盖符号链接：{path}")
    before = path.read_text(encoding="utf-8") if path.exists() else None
    text = before or ""
    index_url = PYPI_INDEXES[mirror]
    body = f'[[index]]\nurl = "{index_url}"\ndefault = true'
    after = _managed_block(text, UV_BEGIN, UV_END, body, official=False)
    try:
        tomllib.loads(after)
    except tomllib.TOMLDecodeError as exc:
        raise MirrorError(f"uv 配置格式无效，未修改 {path}: {exc}") from exc
    mode = stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o644
    return Change(path, before, after, mode)


def _toml_table_span(lines: list[str], table: str) -> tuple[int, int] | None:
    escaped = re.escape(table)
    header = re.compile(rf"^\s*\[{escaped}\]\s*(?:#.*)?$")
    start = next((i for i, line in enumerate(lines) if header.match(line)), None)
    if start is None:
        return None
    end = len(lines)
    for index in range(start + 1, len(lines)):
        if re.match(r"^\s*(?:\[[^\]]+\]|\[\[[^\]]+\]\])\s*(?:#.*)?$", lines[index]):
            end = index
            break
    return start, end


def _remove_marked_line(text: str, marker: str) -> str:
    lines = [line for line in text.splitlines() if marker not in line]
    return "\n".join(lines) + ("\n" if text.endswith("\n") and lines else "")


def cargo_config_text(text: str, mirror: str) -> str:
    try:
        parsed = tomllib.loads(text) if text.strip() else {}
    except tomllib.TOMLDecodeError as exc:
        raise MirrorError(f"Cargo 配置格式无效，未修改：{exc}") from exc

    marker_block = text.count(CARGO_BEGIN) == 1 and text.count(CARGO_END) == 1
    if text.count(CARGO_BEGIN) != text.count(CARGO_END) or text.count(CARGO_BEGIN) > 1:
        raise MirrorError("Cargo 配置中的 NeonMix 管理标记不完整或重复")
    managed_text = ""
    if marker_block:
        block_start = text.index(CARGO_BEGIN)
        block_end = text.index(CARGO_END, block_start) + len(CARGO_END)
        managed_text = text[block_start:block_end]
    managed_crates_table = bool(
        re.search(r"(?m)^\s*\[source\.crates-io\]\s*(?:#.*)?$", managed_text)
    )
    managed_replace_line = any(
        CARGO_KEY_MARKER in line and re.match(r"^\s*replace-with\s*=", line)
        for line in text.splitlines()
    )

    if mirror == "official":
        result = _managed_block(text, CARGO_BEGIN, CARGO_END, "", official=True)
        result = _remove_marked_line(result, CARGO_KEY_MARKER)
        try:
            tomllib.loads(result)
        except tomllib.TOMLDecodeError as exc:
            raise MirrorError(f"移除 Cargo 镜像配置后 TOML 无效：{exc}") from exc
        return result

    source_table = parsed.get("source", {})
    crates_io = source_table.get("crates-io", {}) if isinstance(source_table, dict) else {}
    replacement = crates_io.get("replace-with") if isinstance(crates_io, dict) else None
    custom_source = source_table.get(CARGO_SOURCE) if isinstance(source_table, dict) else None
    if replacement not in (None, CARGO_SOURCE) and not managed_replace_line:
        raise MirrorError(
            "Cargo 已有 [source.crates-io].replace-with；为避免覆盖现有镜像配置，本次未修改。"
        )
    if custom_source is not None and not marker_block:
        raise MirrorError(
            "Cargo 配置已使用 source.neonmix-mirror，但没有 NeonMix 管理标记；请先检查该配置。"
        )

    lines = text.splitlines()
    existing_block_body = (
        f'[source.{CARGO_SOURCE}]\nregistry = "{CARGO_INDEXES[mirror]}"\n\n'
        f'[registries.{CARGO_SOURCE}]\nindex = "{CARGO_INDEXES[mirror]}"'
    )
    if marker_block and managed_crates_table:
        complete_body = (
            f"[source.crates-io]\nreplace-with = \"{CARGO_SOURCE}\" {CARGO_KEY_MARKER}\n\n"
            f"{existing_block_body}"
        )
        result = _managed_block(text, CARGO_BEGIN, CARGO_END, complete_body, official=False)
    else:
        table_span = _toml_table_span(lines, "source.crates-io")
        if table_span is None:
            complete_body = (
                f"[source.crates-io]\nreplace-with = \"{CARGO_SOURCE}\" {CARGO_KEY_MARKER}\n\n"
                f"{existing_block_body}"
            )
            result = _managed_block(text, CARGO_BEGIN, CARGO_END, complete_body, official=False)
        else:
            start, end = table_span
            current_replace_line = next(
                (
                    index
                    for index in range(start + 1, end)
                    if re.match(r"^\s*replace-with\s*=", lines[index])
                ),
                None,
            )
            if current_replace_line is not None:
                if CARGO_KEY_MARKER not in lines[current_replace_line] and replacement != CARGO_SOURCE:
                    raise MirrorError(
                        "Cargo 已有 [source.crates-io].replace-with；为避免覆盖现有镜像配置，本次未修改。"
                    )
                indentation = re.match(r"^\s*", lines[current_replace_line]).group(0)
                lines[current_replace_line] = (
                    f'{indentation}replace-with = "{CARGO_SOURCE}" {CARGO_KEY_MARKER}'
                )
            else:
                lines.insert(end, f'replace-with = "{CARGO_SOURCE}" {CARGO_KEY_MARKER}')
            result = "\n".join(lines)
            result = result + ("\n" if text.endswith("\n") else "")
            block = f"{CARGO_BEGIN}\n{existing_block_body}\n{CARGO_END}"
            if marker_block:
                result = _managed_block(result, CARGO_BEGIN, CARGO_END, existing_block_body, official=False)
            else:
                result = f"{result.rstrip()}\n\n{block}\n"

    try:
        tomllib.loads(result)
    except tomllib.TOMLDecodeError as exc:
        raise MirrorError(f"生成的 Cargo 配置格式无效，未修改：{exc}") from exc
    return result


def _config_home(user: pwd.struct_passwd) -> Path:
    if os.geteuid() == 0 and os.environ.get("SUDO_USER"):
        return Path(user.pw_dir) / ".config"
    return Path(os.environ.get("XDG_CONFIG_HOME", Path(user.pw_dir) / ".config")).expanduser()


def _cargo_home(user: pwd.struct_passwd) -> Path:
    configured = os.environ.get("CARGO_HOME")
    if configured:
        candidate = Path(configured).expanduser()
        root_home = Path(pwd.getpwuid(0).pw_dir).resolve()
        if not (os.geteuid() == 0 and os.environ.get("SUDO_USER") and (candidate == root_home or root_home in candidate.parents)):
            return candidate
    return Path(user.pw_dir) / ".cargo"


def user_config_changes(targets: Iterable[str], mirror: str, user: pwd.struct_passwd) -> list[Change]:
    config_home = _config_home(user)
    changes: list[Change] = []
    if "pip" in targets:
        path = config_home / "pip" / "pip.conf"
        if path.is_symlink():
            raise MirrorError(f"拒绝覆盖符号链接：{path}")
        before = path.read_text(encoding="utf-8") if path.exists() else None
        after = _set_pip_index(before or "", PYPI_INDEXES[mirror])
        changes.append(
            Change(
                path,
                before,
                after,
                stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o644,
                user.pw_uid,
                user.pw_gid,
            )
        )
    if "uv" in targets:
        path = config_home / "uv" / "uv.toml"
        change = uv_config_change(path, mirror)
        change.uid = user.pw_uid
        change.gid = user.pw_gid
        changes.append(change)
    if "cargo" in targets:
        if mirror != "official" and mirror not in CARGO_INDEXES:
            raise MirrorError("Cargo 目前支持 official、ustc 和 tsinghua；aliyun 的 crates.io 索引同步过旧。")
        path = _cargo_home(user) / "config.toml"
        if path.is_symlink():
            raise MirrorError(f"拒绝覆盖符号链接：{path}")
        before = path.read_text(encoding="utf-8") if path.exists() else None
        after = cargo_config_text(before or "", mirror)
        if before != after:
            changes.append(
                Change(
                    path,
                    before,
                    after,
                    stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o644,
                    user.pw_uid,
                    user.pw_gid,
                )
            )
    return [change for change in changes if change.before != change.after]


def _backup_paths(path: Path) -> tuple[Path, Path]:
    return Path(f"{path}{BACKUP_SUFFIX}"), Path(f"{path}{ABSENT_SUFFIX}")


def _ensure_parent(path: Path, user: pwd.struct_passwd) -> None:
    missing: list[Path] = []
    current = path.parent
    while not current.exists() and current != current.parent:
        missing.append(current)
        current = current.parent
    path.parent.mkdir(parents=True, exist_ok=True)
    if os.geteuid() == 0 and os.environ.get("SUDO_USER"):
        for directory in missing:
            os.chown(directory, user.pw_uid, user.pw_gid)


def _write_preserving(path: Path, text: str, mode: int, user: pwd.struct_passwd) -> None:
    _ensure_parent(path, user)
    if path.is_symlink():
        raise MirrorError(f"拒绝覆盖符号链接：{path}")
    existed = path.exists()
    previous_mode = stat.S_IMODE(path.stat().st_mode) if existed else mode
    try:
        with path.open("w", encoding="utf-8", newline="") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(path, previous_mode)
    except OSError as exc:
        raise MirrorError(f"写入 {path} 失败：{exc}") from exc


def _write_change(change: Change, text: str, user: pwd.struct_passwd) -> None:
    _write_preserving(change.path, text, change.mode or 0o644, user)
    if change.uid is not None and change.gid is not None and os.geteuid() == 0:
        os.chown(change.path, change.uid, change.gid)


def _snapshot(path: Path, user: pwd.struct_passwd, uid: int | None, gid: int | None) -> None:
    backup, absent = _backup_paths(path)
    if backup.exists() or absent.exists():
        return
    _ensure_parent(backup, user)
    if path.exists():
        shutil.copy2(path, backup)
        if uid is not None and gid is not None and os.geteuid() == 0:
            os.chown(backup, uid, gid)
    else:
        _write_preserving(absent, "Originally absent\n", 0o600, user)
        if uid is not None and gid is not None and os.geteuid() == 0:
            os.chown(absent, uid, gid)


def _diff(change: Change) -> str:
    before = (change.before or "").splitlines(keepends=True)
    after = (change.after or "").splitlines(keepends=True)
    return "".join(
        difflib.unified_diff(before, after, fromfile=str(change.path), tofile=str(change.path) + " (new)")
    )


def show_plan(changes: Iterable[Change], dry_run: bool) -> None:
    changes = list(changes)
    if not changes:
        print("没有需要修改的配置。")
        return
    print("预览配置变更：" if dry_run else "将修改以下配置：")
    for change in changes:
        print(f"\n--- {change.path}")
        if dry_run:
            print(_diff(change), end="")
        else:
            print("  将保留原始备份；之后可用 restore 命令恢复。")


def apply_changes(changes: Iterable[Change], user: pwd.struct_passwd, dry_run: bool) -> None:
    changes = list(changes)
    show_plan(changes, dry_run)
    if dry_run:
        return
    for change in changes:
        _snapshot(change.path, user, change.uid, change.gid)
        if change.after is None:
            if change.path.exists():
                change.path.unlink()
        else:
            _write_change(change, change.after, user)


def restore_changes(targets: Iterable[str], distro: Distro, user: pwd.struct_passwd) -> list[Change]:
    paths: list[Path] = []
    if "system" in targets:
        kind = native_system_kind(distro)
        if kind == "apt":
            apt_root = Path("/etc/apt")
            apt_sources = apt_root / "sources.list"
            paths.append(apt_sources)
            source_dir = apt_root / "sources.list.d"
            if source_dir.is_dir():
                paths.extend(sorted(source_dir.glob("*.list")))
                paths.extend(sorted(source_dir.glob("*.sources")))
                for backup in sorted(source_dir.glob(f"*{BACKUP_SUFFIX}")):
                    original_name = backup.name[: -len(BACKUP_SUFFIX)]
                    if original_name.endswith((".list", ".sources")):
                        paths.append(backup.with_name(original_name))
        elif kind == "fedora-dnf5":
            paths.append(Path("/etc/dnf/repos.override.d/99-neonmix-mirror.repo"))
        elif kind == "arch":
            paths.append(Path("/etc/pacman.d/mirrorlist"))
        else:
            raise MirrorError(
                f"检测到 {distro.name}；当前 system 回退支持 Ubuntu/Debian APT、Fedora 44+ DNF5 和 Arch Linux。"
            )
    config_home = _config_home(user)
    if "pip" in targets:
        paths.append(config_home / "pip" / "pip.conf")
    if "uv" in targets:
        paths.append(config_home / "uv" / "uv.toml")
    if "cargo" in targets:
        paths.append(_cargo_home(user) / "config.toml")

    changes: list[Change] = []
    for path in dict.fromkeys(paths):
        backup, absent = _backup_paths(path)
        if not backup.is_file() and not absent.is_file():
            continue
        if path.is_symlink():
            raise MirrorError(f"拒绝恢复符号链接：{path}")
        if backup.is_file():
            before = path.read_text(encoding="utf-8", errors="replace") if path.is_file() else None
            after = backup.read_text(encoding="utf-8", errors="replace")
            mode = stat.S_IMODE(backup.stat().st_mode)
        elif absent.is_file():
            before = path.read_text(encoding="utf-8", errors="replace") if path.is_file() else None
            after = None
            mode = None
        else:
            continue
        if before != after:
            is_user_config = not path.is_relative_to(Path("/etc/apt"))
            changes.append(
                Change(
                    path,
                    before,
                    after,
                    mode,
                    user.pw_uid if is_user_config else None,
                    user.pw_gid if is_user_config else None,
                )
            )
    return changes


def supported_targets(targets: tuple[str, ...], distro: Distro) -> tuple[str, ...]:
    if "system" not in targets or native_system_kind(distro):
        return targets
    if set(targets) != set(TARGETS):
        raise MirrorError(
            f"检测到 {distro.name}（包管理器：{distro.package_manager}）；"
            "system 源切换仅支持 Ubuntu/Debian 系列 APT。"
        )
    print(f"system: 跳过 {distro.name}，当前尚未配置该发行版的原生软件源。")
    return tuple(item for item in targets if item != "system")


def print_detection(distro: Distro) -> None:
    print(f"系统：{distro.name}")
    print(f"ID：{distro.os_id}")
    print(f"ID_LIKE：{' '.join(distro.id_like) or '(未设置)'}")
    print(f"版本：{distro.version}")
    print(f"代号：{distro.codename or '(未设置)'}")
    print(f"包管理器：{distro.package_manager}")
    system_kind = native_system_kind(distro)
    if system_kind == "apt":
        print(f"系统源支持：APT（{distro.apt_family} 系列）")
    elif system_kind == "fedora-dnf5":
        print("系统源支持：Fedora DNF5（Fedora 44+）")
    elif system_kind == "arch":
        print("系统源支持：Arch pacman")
    else:
        print("系统源支持：当前脚本未配置此发行版的原生源切换")


def print_mirrors() -> None:
    print("支持的镜像源：")
    for key, details in MIRRORS.items():
        print(f"  {key:<9} {details['label']}")
    print("\n源类型：system（Ubuntu/Debian APT、Fedora 44+ DNF5、Arch pacman）、pip、uv、cargo")
    print("Cargo 可选源：official、ustc、tsinghua；Aliyun crates.io 索引未启用。")
    print("Cargo 与 Python 源可单独使用，不依赖 Linux 发行版类型。")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="检测 Linux 发行版并切换 system、pip、uv、Cargo 镜像源。",
        epilog=(
            "示例：\n"
            "  %(prog)s detect\n"
            "  tools/dev python3 %(prog)s set ustc --for pip,uv,cargo\n"
            "  sudo python3 %(prog)s set tsinghua --for system\n"
            "  %(prog)s set official --for all\n"
            "  %(prog)s restore --for all"
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("detect", help="显示发行版、版本和包管理器")
    subparsers.add_parser("list", help="列出镜像站和支持的源类型")
    for command in ("set", "restore"):
        sub = subparsers.add_parser(
            command,
            help="切换镜像源" if command == "set" else "恢复脚本修改前的配置",
        )
        if command == "set":
            sub.add_argument("mirror", help="official、ustc、tsinghua/tuna 或 aliyun")
        sub.add_argument("--for", dest="targets", default="all", help="逗号分隔的源类型；默认 all")
        sub.add_argument("--dry-run", action="store_true", help="仅显示将要修改的配置")
        if command == "set":
            sub.add_argument(
                "--include-security",
                action="store_true",
                help="也切换 APT 安全更新源（镜像可能存在同步延迟）",
            )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    distro = detect_distro()
    if args.command == "detect":
        print_detection(distro)
        return 0
    if args.command == "list":
        print_mirrors()
        return 0

    targets = parse_targets(args.targets)
    user = _current_user()
    effective_targets = supported_targets(targets, distro)
    if args.command == "restore":
        if "system" in effective_targets and os.geteuid() != 0 and not args.dry_run:
            raise MirrorError("恢复系统软件源需要 root 权限；请使用 sudo 运行 system 目标。")
        changes = restore_changes(effective_targets, distro, user)
        apply_changes(changes, user, args.dry_run)
        return 0

    mirror = normalize_mirror(args.mirror)
    if args.include_security and "system" not in targets:
        raise MirrorError("--include-security 只适用于 --for 中包含 system 的情况。")
    if "cargo" in effective_targets and mirror not in {"official", *CARGO_INDEXES}:
        if set(targets) == set(TARGETS):
            print("cargo: 跳过 aliyun，该站 crates.io 索引同步过旧；其余支持的目标继续执行。")
            effective_targets = tuple(item for item in effective_targets if item != "cargo")
        else:
            raise MirrorError("Cargo 目前支持 official、ustc 和 tsinghua；aliyun 的 crates.io 索引同步过旧。")
    if "system" in effective_targets and os.geteuid() != 0 and not args.dry_run:
        raise MirrorError("修改系统软件源需要 root 权限；请使用 sudo 运行 system 目标。")

    changes: list[Change] = []
    system_change_count = 0
    if "system" in effective_targets:
        system_config_changes = build_system_changes(distro, mirror, args.include_security)
        system_change_count = len(system_config_changes)
        changes.extend(system_config_changes)
    changes.extend(user_config_changes(effective_targets, mirror, user))
    apply_changes(changes, user, args.dry_run)
    if "system" in effective_targets and system_change_count == 0:
        if native_system_kind(distro) == "apt":
            print("system: 未发现可识别的 Ubuntu/Debian 官方 APT 源地址，未修改第三方源。")
    if "system" in effective_targets and system_change_count and not args.dry_run:
        if native_system_kind(distro) == "apt":
            print("系统源配置已更新；生效前请运行：sudo apt-get update")
        elif native_system_kind(distro) == "fedora-dnf5":
            print("系统源配置已更新；生效前请运行：sudo dnf makecache")
        elif native_system_kind(distro) == "arch":
            print("系统源配置已更新；生效前请运行：sudo pacman -Syy")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except MirrorError as exc:
        print(f"错误：{exc}", file=sys.stderr)
        raise SystemExit(2)
    except (OSError, UnicodeError) as exc:
        print(f"错误：{exc}", file=sys.stderr)
        raise SystemExit(2)
