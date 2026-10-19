#!/usr/bin/env python3
"""Exercise real Sparkle signatures with ephemeral keys, including tampering."""
import base64
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent
TOOLS = Path(sys.argv[1]).resolve()
SIGN = TOOLS / "bin/sign_update"
NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"


def invoke(command, *, env=None, succeeds=True):
    result = subprocess.run(command, env=env, capture_output=True)
    if (result.returncode == 0) != succeeds:
        raise AssertionError(f"Unexpected result for {Path(str(command[0])).name}: {result.stderr.decode()}")
    return result


with tempfile.TemporaryDirectory(prefix="cowchat-appcast-test-") as temporary:
    work = Path(temporary)
    seed = base64.b64encode(os.urandom(32)).decode()
    key = work / "key"
    key.write_text(seed)
    key.chmod(0o600)
    public = subprocess.run(["swift", str(ROOT / "sparkle-key.swift")], input=seed.encode(), capture_output=True, check=True).stdout.decode().strip()
    info = work / "Info.plist"
    info.write_bytes(plistlib.dumps({
        "CFBundleVersion": "0.9.10", "CFBundleShortVersionString": "0.9.10",
        "SUPublicEDKey": public, "SURequireSignedFeed": True,
        "SUVerifyUpdateBeforeExtraction": True, "LSMinimumSystemVersion": "13.0",
    }))
    dmg = work / "Cowchat-0.9.10.dmg"
    dmg.write_bytes(b"final stapled DMG fixture\n")
    feed = work / "appcast.xml"
    command = [sys.executable, str(ROOT / "sign-appcast.py"), "--dmg", str(dmg), "--info-plist", str(info), "--tools", str(TOOLS), "--output", str(feed)]
    environment = dict(os.environ, SPARKLE_PRIVATE_KEY=seed)
    invoke(command, env=environment)
    tree = ET.parse(feed)
    enclosure = tree.find("channel/item/enclosure")
    assert enclosure.attrib["length"] == str(dmg.stat().st_size)
    assert tree.find(f"channel/item/{{{NS}}}version").text == "0.9.10"
    signature = enclosure.attrib[f"{{{NS}}}edSignature"]
    invoke([str(SIGN), "--ed-key-file", str(key), "--verify", str(dmg), signature])
    invoke([str(SIGN), "--ed-key-file", str(key), "--verify", str(feed)])
    dmg.write_bytes(dmg.read_bytes() + b"tampered")
    invoke([str(SIGN), "--ed-key-file", str(key), "--verify", str(dmg), signature], succeeds=False)
    feed.write_bytes(feed.read_bytes().replace(b"Cowchat updates", b"Malicious feed!"))
    invoke([str(SIGN), "--ed-key-file", str(key), "--verify", str(feed)], succeeds=False)
    # Both mismatch and missing key must fail before producing a feed.
    feed.unlink()
    invoke(command, env=dict(environment, SPARKLE_PRIVATE_KEY=base64.b64encode(os.urandom(32)).decode()), succeeds=False)
    assert not feed.exists()
    invoke(command, env=dict(environment, SPARKLE_PRIVATE_KEY=""), succeeds=False)
    assert not feed.exists()
print("Real Sparkle archive/feed signing, tampering, wrong-key, and missing-key checks passed.")
