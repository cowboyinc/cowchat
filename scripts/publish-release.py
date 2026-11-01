#!/usr/bin/env python3
"""Publish complete release assets through a draft; published bytes are immutable."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile


def gh(*args, capture=False):
    return subprocess.run(["gh", *args], check=True, stdout=subprocess.PIPE if capture else None).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--notes", type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"v\d+\.\d+\.\d+", args.tag):
        raise SystemExit("Invalid release tag")
    version = args.tag[1:]
    names = [f"cowchat-{args.tag}-{target}.tar.gz" for target in (
        "aarch64-apple-darwin", "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"
    )] + [f"Cowchat-{version}.dmg", "appcast.xml"]
    for name in names:
        if not (args.dist / name).is_file() or not (args.dist / name).stat().st_size:
            raise SystemExit(f"Missing release asset: {name}")
    # API errors must not be mistaken for a missing release.
    releases = json.loads(gh("api", f"repos/{args.repo}/releases", "--paginate", "--slurp", capture=True))
    all_releases = [release for page in releases for release in page]
    release = next((release for release in all_releases if release["tag_name"] == args.tag), None)
    # GitHub's latest flag otherwise allows an older tag published later to
    # replace the stable signed feed. Compare numeric components, never strings.
    version_tuple = tuple(map(int, version.split(".")))
    stable_versions = [tuple(map(int, entry["tag_name"][1:].split(".")))
                       for entry in all_releases
                       if not entry["draft"] and not entry.get("prerelease", False)
                       and re.fullmatch(r"v\d+\.\d+\.\d+", entry["tag_name"])]
    latest = not stable_versions or version_tuple >= max(stable_versions)
    if release and not release["draft"]:
        present = {asset["name"] for asset in release["assets"]}
        if not set(names).issubset(present):
            raise SystemExit("Published release is incomplete; refusing to mutate it")
        with tempfile.TemporaryDirectory(prefix="cowchat-release-check-") as temporary:
            for name in names:
                gh("release", "download", args.tag, "--repo", args.repo, "--pattern", name, "--dir", temporary)
                if (Path(temporary) / name).read_bytes() != (args.dist / name).read_bytes():
                    raise SystemExit(f"Published asset differs; refusing replacement: {name}")
        print("Published release already contains identical assets.")
        return
    if not release:
        gh("release", "create", args.tag, "--repo", args.repo, "--verify-tag", "--draft",
           "--title", f"Cowchat {args.tag}", "--notes-file", str(args.notes))
    gh("release", "upload", args.tag, "--repo", args.repo, "--clobber", *(str(args.dist / name) for name in names))
    # All assets, including the signed feed, become visible together.
    gh("release", "edit", args.tag, "--repo", args.repo, "--draft=false", "--prerelease=false", f"--latest={str(latest).lower()}",
       "--title", f"Cowchat {args.tag}", "--notes-file", str(args.notes))


if __name__ == "__main__":
    main()
