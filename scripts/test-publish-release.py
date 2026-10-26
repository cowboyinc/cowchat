#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("publish_release", Path(__file__).with_name("publish-release.py"))
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class PublicationTests(unittest.TestCase):
    def exercise(self, state, *, different=False, fail_upload=False, other_version=None):
        with tempfile.TemporaryDirectory() as temporary:
            dist = Path(temporary) / "dist"
            dist.mkdir()
            names = [f"cowchat-v0.9.10-{target}.tar.gz" for target in (
                "aarch64-apple-darwin", "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"
            )] + ["Cowchat-0.9.10.dmg", "appcast.xml"]
            for name in names:
                (dist / name).write_bytes(name.encode())
            notes = Path(temporary) / "notes"
            notes.write_text("Release notes")
            calls = []

            def fake_gh(*args, capture=False):
                calls.append(args)
                if args[0] == "api":
                    release = {"tag_name": "v0.9.10", "draft": state == "draft", "assets": [{"name": name} for name in names]}
                    releases = [] if state == "absent" else [release]
                    if other_version:
                        releases.append({"tag_name": other_version, "draft": False, "prerelease": False, "assets": []})
                    return json.dumps([releases]).encode()
                if args[:2] == ("release", "download"):
                    name = args[args.index("--pattern") + 1]
                    destination = Path(args[args.index("--dir") + 1])
                    (destination / name).write_bytes(b"different" if different else name.encode())
                if args[:2] == ("release", "upload") and fail_upload:
                    raise subprocess.CalledProcessError(1, ["gh"])
                return b""

            argv = ["publish-release.py", "--repo", "cowboyinc/cowchat", "--tag", "v0.9.10", "--dist", str(dist), "--notes", str(notes)]
            with patch.object(sys, "argv", argv), patch.object(publisher, "gh", fake_gh):
                if different:
                    with self.assertRaisesRegex(SystemExit, "refusing replacement"):
                        publisher.main()
                elif fail_upload:
                    with self.assertRaises(subprocess.CalledProcessError):
                        publisher.main()
                else:
                    publisher.main()
            return calls

    def test_new_release_publishes_only_after_upload(self):
        calls = self.exercise("absent")
        mutations = [call for call in calls if call[:2] in (("release", "create"), ("release", "upload"), ("release", "edit"))]
        self.assertEqual([call[1] for call in mutations], ["create", "upload", "edit"])
        self.assertIn("--draft", mutations[0])
        self.assertIn("--draft=false", mutations[-1])

    def test_older_tag_does_not_replace_newer_feed(self):
        calls = self.exercise("absent", other_version="v0.9.11")
        self.assertIn("--latest=false", calls[-1])
        self.assertNotIn("--latest=true", calls[-1])

    def test_numeric_versions_promote_ten_after_nine(self):
        calls = self.exercise("absent", other_version="v0.9.9")
        self.assertIn("--latest=true", calls[-1])

    def test_failed_upload_leaves_release_private(self):
        calls = self.exercise("absent", fail_upload=True)
        self.assertFalse(any(call[:2] == ("release", "edit") for call in calls))

    def test_existing_draft_resumes(self):
        calls = self.exercise("draft")
        self.assertFalse(any(call[:2] == ("release", "create") for call in calls))
        self.assertEqual(calls[-1][:2], ("release", "edit"))

    def test_identical_publication_is_read_only(self):
        calls = self.exercise("published")
        self.assertFalse(any(call[:2] in (("release", "upload"), ("release", "edit"), ("release", "create")) for call in calls))

    def test_different_published_bytes_cannot_be_replaced(self):
        calls = self.exercise("published", different=True)
        self.assertFalse(any(call[:2] in (("release", "upload"), ("release", "edit")) for call in calls))


unittest.main()
