#!/usr/bin/env python3
"""Check the cowchat-core pin in `cargo metadata` output.

Every git dependency must be cowchat-core at one full commit, and the
cowchat-core and cowchat-client packages must both come from it. With
--on-main, that commit must also be on cowchat-core's main branch, so a
build never ships protocol code that hasn't merged there.
"""
import argparse
import json
import re
import subprocess
import sys
import tempfile

CORE = "https://github.com/cowboyinc/cowchat-core.git"
FULL_COMMIT = re.compile(r"[0-9a-f]{40}")
REQUIRED = ("cowchat-core", "cowchat-client")


class PinError(Exception):
    pass


def core_pin(metadata):
    revs = set()
    found = set()
    for package in metadata["packages"]:
        name = package.get("name")
        source = package.get("source") or ""
        if name in REQUIRED:
            if not source.startswith("git+"):
                raise PinError(f"{name} must come from the cowchat-core git pin, not {source or 'a local path'}")
            found.add(name)
        if not source.startswith("git+"):
            continue
        url, _, rest = source[len("git+"):].partition("?")
        if url != CORE:
            raise PinError(f"git dependency other than cowchat-core: {source}")
        query, _, commit = rest.partition("#")
        rev = query[len("rev="):] if query.startswith("rev=") else ""
        if not FULL_COMMIT.fullmatch(rev) or rev != commit:
            raise PinError(f"cowchat-core must be pinned to a full commit with rev=: {source}")
        revs.add(rev)
    for name in REQUIRED:
        if name not in found:
            raise PinError(f"no cowchat-core git dependency found for {name}")
    if len(revs) > 1:
        raise PinError(f"cowchat-core is pinned to more than one commit: {sorted(revs)}")
    return revs.pop()


def is_on_main(url, rev):
    with tempfile.TemporaryDirectory() as repo:
        def git(*args):
            return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)

        git("init", "-q")
        fetched = git("fetch", "-q", "--filter=blob:none", url, "refs/heads/main:refs/remotes/core/main")
        if fetched.returncode != 0:
            raise PinError(f"could not fetch cowchat-core main: {fetched.stderr.strip()}")
        if git("cat-file", "-e", f"{rev}^{{commit}}").returncode != 0:
            return False
        return git("merge-base", "--is-ancestor", rev, "refs/remotes/core/main").returncode == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metadata", help="path to `cargo metadata --all-features --format-version 1` output")
    parser.add_argument("--on-main", action="store_true", help="also require the pin to be on cowchat-core main")
    args = parser.parse_args()
    with open(args.metadata) as handle:
        metadata = json.load(handle)
    try:
        rev = core_pin(metadata)
        if args.on_main and not is_on_main(CORE, rev):
            raise PinError(f"cowchat-core {rev} is not on cowchat-core main; merge the core change first")
    except PinError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"cowchat-core pinned to {rev}" + (" (on main)" if args.on_main else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
