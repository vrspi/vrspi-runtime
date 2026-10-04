#!/usr/bin/env python3
"""Builds the update manifest that `vrspi update` and install.sh both read.

Both read the same file on purpose: an install and an update can then never
disagree about what the latest release is.

Digests are computed from the asset bytes as published, not from the build
job's own output, so a corrupted upload is caught here rather than by a user
halfway through an update.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys

# Manifest platform key -> published asset file name.
PLATFORMS = {
    "linux-x86_64": "vrspi-linux-x86_64",
    "linux-aarch64": "vrspi-linux-aarch64",
    "macos-x86_64": "vrspi-macos-x86_64",
    "macos-aarch64": "vrspi-macos-aarch64",
    "windows-x86_64": "vrspi-windows-x86_64.zip",
}


def protocol_version(source: pathlib.Path) -> int:
    """Reads the wire protocol version the release was built with."""
    match = re.search(
        r"pub const PROTOCOL_VERSION: u32 = (\d+);", source.read_text(encoding="utf-8")
    )
    if match is None:
        raise SystemExit(f"PROTOCOL_VERSION not found in {source}")
    return int(match.group(1))


def build(version: str, protocol: int, assets_dir: pathlib.Path, base_url: str, notes: str) -> dict:
    assets: dict[str, str] = {}
    digests: dict[str, str] = {}
    base = f"{base_url.rstrip('/')}/v{version}"
    for key, name in PLATFORMS.items():
        path = assets_dir / name
        if not path.exists():
            # A platform this release did not publish is simply absent, so the
            # installer can say "no build for your platform" instead of
            # downloading a file that is not there.
            continue
        assets[key] = f"{base}/{name}"
        digests[key] = hashlib.sha256(path.read_bytes()).hexdigest()
    if not assets:
        raise SystemExit(f"no release assets found in {assets_dir}; refusing to publish an empty manifest")
    return {
        "version": version,
        "protocol": protocol,
        "notes": notes,
        "assets": assets,
        "sha256": digests,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="release version, without a v prefix")
    parser.add_argument("--assets", required=True, type=pathlib.Path, help="directory of downloaded release assets")
    parser.add_argument(
        "--base-url",
        required=True,
        help="URL the versioned asset directories are served under, e.g. https://vrspi.com/runtime/releases",
    )
    parser.add_argument("--notes", type=pathlib.Path, help="file holding the release notes")
    parser.add_argument("--wire", type=pathlib.Path, default=pathlib.Path("src/protocol/wire.rs"))
    parser.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()

    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        raise SystemExit(f"version must look like 0.9.0, got {args.version!r}")

    notes = args.notes.read_text(encoding="utf-8") if args.notes and args.notes.exists() else ""
    manifest = build(args.version, protocol_version(args.wire), args.assets, args.base_url, notes)
    args.output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output} listing {len(manifest['assets'])} platform(s) for v{args.version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
