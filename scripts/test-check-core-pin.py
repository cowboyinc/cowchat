#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("check_core_pin", Path(__file__).with_name("check-core-pin.py"))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

CORE = "https://github.com/cowboyinc/cowchat-core.git"
REV = "43822264de6d0921ee8bf294ef1713a44f7cf7e5"


def package(source):
    return {"name": "x", "source": source}


def git_source(rev, url=CORE, commit=None):
    return f"git+{url}?rev={rev}#{commit or rev}"


class PinTests(unittest.TestCase):
    def test_one_core_rev_is_the_pin(self):
        metadata = {"packages": [
            package(None),
            package("registry+https://github.com/rust-lang/crates.io-index"),
            package(git_source(REV)),
            package(git_source(REV)),
        ]}
        self.assertEqual(checker.core_pin(metadata), REV)

    def test_a_git_source_other_than_core_is_refused(self):
        metadata = {"packages": [package(git_source(REV, url="https://github.com/someone/else.git"))]}
        with self.assertRaisesRegex(checker.PinError, "someone/else"):
            checker.core_pin(metadata)

    def test_two_core_revs_are_refused(self):
        other = "0cbdf07c" + REV[8:]
        metadata = {"packages": [package(git_source(REV)), package(git_source(other))]}
        with self.assertRaisesRegex(checker.PinError, "more than one"):
            checker.core_pin(metadata)

    def test_a_branch_or_short_rev_is_refused(self):
        for rev in ("main", REV[:8]):
            metadata = {"packages": [package(git_source(rev, commit=REV))]}
            with self.assertRaisesRegex(checker.PinError, "full commit"):
                checker.core_pin(metadata)

    def test_no_core_dependency_is_refused(self):
        with self.assertRaisesRegex(checker.PinError, "no cowchat-core"):
            checker.core_pin({"packages": [package(None)]})


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, text=True).stdout.strip()


class OnMainTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.repo = root / "core"
        subprocess.run(["git", "init", "-q", "-b", "main", str(self.repo)], check=True)
        git(self.repo, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "one")
        self.on_main = git(self.repo, "rev-parse", "HEAD")
        git(self.repo, "checkout", "-q", "-b", "feature")
        git(self.repo, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "two")
        self.off_main = git(self.repo, "rev-parse", "HEAD")
        self.url = self.repo.as_uri()

    def tearDown(self):
        self.temporary.cleanup()

    def test_a_commit_on_main_passes(self):
        self.assertTrue(checker.is_on_main(self.url, self.on_main))

    def test_a_commit_only_on_a_branch_fails(self):
        self.assertFalse(checker.is_on_main(self.url, self.off_main))

    def test_an_unknown_commit_fails(self):
        self.assertFalse(checker.is_on_main(self.url, "f" * 40))


if __name__ == "__main__":
    unittest.main()
