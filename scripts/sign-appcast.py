#!/usr/bin/env python3
"""Sign final notarized DMG bytes and the appcast, without accessing Keychain."""
import argparse
import base64
import os
from pathlib import Path
import plistlib
import re
import subprocess
import tempfile
import xml.etree.ElementTree as ET

SPARKLE = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", SPARKLE)


def run(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, **kwargs).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dmg", type=Path, required=True)
    parser.add_argument("--info-plist", type=Path, required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    key = os.environ.get("SPARKLE_PRIVATE_KEY", "").strip()
    if not key:
        raise ValueError("SPARKLE_PRIVATE_KEY is required; Keychain fallback is forbidden")
    info = plistlib.loads(args.info_plist.read_bytes())
    version = info["CFBundleVersion"]
    short = info["CFBundleShortVersionString"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", version) or version != short:
        raise ValueError("App build and short versions must match the release version")
    if args.dmg.name != f"Cowchat-{version}.dmg":
        raise ValueError("DMG filename does not match the embedded app version")
    public = run("swift", str(Path(__file__).with_name("sparkle-key.swift")), input=key.encode()).decode().strip()
    if public != info["SUPublicEDKey"]:
        raise ValueError("Sparkle signing key does not match embedded SUPublicEDKey")
    if info.get("SURequireSignedFeed") is not True or info.get("SUVerifyUpdateBeforeExtraction") is not True:
        raise ValueError("The app must require signed feeds and archive validation before extraction")
    if len(base64.b64decode(public, validate=True)) != 32:
        raise ValueError("Invalid Sparkle public key")
    tool = str(args.tools.resolve() / "bin/sign_update")
    with tempfile.TemporaryDirectory(prefix="cowchat-sparkle-") as temporary:
        private = Path(temporary) / "key"
        private.write_text(key)
        private.chmod(0o600)
        signature = run(tool, "--ed-key-file", str(private), "-p", str(args.dmg.resolve())).decode().strip()
        run(tool, "--ed-key-file", str(private), "--verify", str(args.dmg.resolve()), signature)
        rss = ET.Element("rss", version="2.0")
        channel = ET.SubElement(rss, "channel")
        ET.SubElement(channel, "title").text = "Cowchat updates"
        item = ET.SubElement(channel, "item")
        ET.SubElement(item, "title").text = f"Cowchat {short}"
        ET.SubElement(item, f"{{{SPARKLE}}}version").text = version
        ET.SubElement(item, f"{{{SPARKLE}}}shortVersionString").text = short
        ET.SubElement(item, f"{{{SPARKLE}}}minimumSystemVersion").text = info["LSMinimumSystemVersion"]
        # The app is arm64-only; Sparkle skips this update on Intel Macs.
        ET.SubElement(item, f"{{{SPARKLE}}}hardwareRequirements").text = "arm64"
        ET.SubElement(item, "enclosure", {
            "url": f"https://github.com/cowboyinc/cowchat/releases/download/v{version}/{args.dmg.name}",
            "length": str(args.dmg.stat().st_size), "type": "application/octet-stream",
            f"{{{SPARKLE}}}edSignature": signature,
        })
        candidate = Path(temporary) / "appcast.xml"
        ET.ElementTree(rss).write(candidate, encoding="utf-8", xml_declaration=True)
        run(tool, "--ed-key-file", str(private), str(candidate))
        run(tool, "--ed-key-file", str(private), "--verify", str(candidate))
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_bytes(candidate.read_bytes())
    print("Signed and verified final DMG and appcast.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, subprocess.CalledProcessError) as error:
        # Never echo subprocess output: signing tools can include sensitive input.
        raise SystemExit(f"Appcast signing failed: {error if not isinstance(error, subprocess.CalledProcessError) else 'signing tool rejected input'}")
