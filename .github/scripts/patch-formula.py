#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///

"""Patch a Homebrew formula with new version, URLs, and SHA256 checksums."""

import argparse
import hashlib
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def patch_formula(formula_path: Path, version: str, artifacts_dir: Path) -> None:
    pkg = "ansible-password-agent"
    tag = f"{pkg}/v{version}"
    encoded_tag = tag.replace("/", "%2F")
    github = "https://github.com/vrischmann/mytools/releases/download"

    # Archive filenames match the build matrix naming convention.
    macos_archive = artifacts_dir / f"{pkg}-{version}-aarch64-apple-darwin.tar.gz"
    linux_archive = artifacts_dir / f"{pkg}-{version}-x86_64-unknown-linux-gnu.tar.gz"

    macos_url = f"{github}/{encoded_tag}/{pkg}-{version}-aarch64-apple-darwin.tar.gz"
    linux_url = (
        f"{github}/{encoded_tag}/{pkg}-{version}-x86_64-unknown-linux-gnu.tar.gz"
    )

    if not macos_archive.exists():
        raise SystemExit(f"macOS archive not found: {macos_archive}")
    if not linux_archive.exists():
        raise SystemExit(f"Linux archive not found: {linux_archive}")

    macos_sha = sha256(macos_archive)
    linux_sha = sha256(linux_archive)

    lines = formula_path.read_text().splitlines(keepends=True)
    result: list[str] = []
    block: str | None = None  # 'macos', 'linux', or None

    for line in lines:
        stripped = line.strip()

        if "on_macos do" in stripped:
            block = "macos"
        elif "on_linux do" in stripped:
            block = "linux"
        elif stripped == "end" and block:
            block = None

        if stripped.startswith('version "'):
            result.append(f'  version "{version}"\n')
        elif block == "macos" and stripped.startswith('url "'):
            result.append(f'    url "{macos_url}"\n')
        elif block == "macos" and stripped.startswith('sha256 "'):
            result.append(f'    sha256 "{macos_sha}"\n')
        elif block == "linux" and stripped.startswith('url "'):
            result.append(f'      url "{linux_url}"\n')
        elif block == "linux" and stripped.startswith('sha256 "'):
            result.append(f'      sha256 "{linux_sha}"\n')
        else:
            result.append(line)

    formula_path.write_text("".join(result))
    print(f"Patched {formula_path} → version {version}")
    print(f"  macOS:  sha256={macos_sha}")
    print(f"  Linux:  sha256={linux_sha}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Patch a Homebrew formula")
    parser.add_argument(
        "--formula", required=True, type=Path, help="Path to the formula .rb file"
    )
    parser.add_argument("--version", required=True, help="New version string")
    parser.add_argument(
        "--artifacts",
        required=True,
        type=Path,
        help="Directory containing built .tar.gz archives",
    )
    args = parser.parse_args()
    patch_formula(args.formula, args.version, args.artifacts)


if __name__ == "__main__":
    main()
