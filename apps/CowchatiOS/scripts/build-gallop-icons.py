#!/usr/bin/env python3
"""Generate Resources/GallopIcons.xcassets from the Mac app's Gallop SVGs.

Each SVG becomes a template-rendered imageset that keeps its vector data, named after the file, so
GallopIcon's raw values load on iOS with UIImage(named:). Rerunning with unchanged SVGs changes nothing;
imagesets whose SVG is gone are removed.
"""

import json
import shutil
import sys
from pathlib import Path

IOS_ROOT = Path(__file__).resolve().parent.parent
SVG_DIR = IOS_ROOT.parent / "CowchatKit" / "Sources" / "CowchatKit" / "Bundled" / "Icons" / "svg"
CATALOG = IOS_ROOT / "Resources" / "GallopIcons.xcassets"
INFO = {"author": "xcode", "version": 1}


def write_json(path: Path, value: dict) -> None:
    text = json.dumps(value, indent=2, sort_keys=True) + "\n"
    if not path.exists() or path.read_text() != text:
        path.write_text(text)


def main() -> int:
    svgs = sorted(SVG_DIR.glob("*.svg"))
    if not svgs:
        print(f"no SVGs in {SVG_DIR}", file=sys.stderr)
        return 1

    CATALOG.mkdir(parents=True, exist_ok=True)
    write_json(CATALOG / "Contents.json", {"info": INFO})

    wanted = {svg.stem + ".imageset" for svg in svgs}
    for entry in CATALOG.iterdir():
        if entry.is_dir() and entry.name not in wanted:
            shutil.rmtree(entry)

    for svg in svgs:
        imageset = CATALOG / (svg.stem + ".imageset")
        imageset.mkdir(exist_ok=True)
        for entry in imageset.iterdir():
            if entry.name not in {svg.name, "Contents.json"}:
                entry.unlink()
        target = imageset / svg.name
        data = svg.read_bytes()
        if not target.exists() or target.read_bytes() != data:
            target.write_bytes(data)
        write_json(imageset / "Contents.json", {
            "images": [{"filename": svg.name, "idiom": "universal"}],
            "info": INFO,
            "properties": {
                "preserves-vector-representation": True,
                "template-rendering-intent": "template",
            },
        })

    print(f"{len(svgs)} icons in {CATALOG.relative_to(IOS_ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
