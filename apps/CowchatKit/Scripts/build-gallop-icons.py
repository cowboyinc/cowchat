#!/usr/bin/env python3
"""Generate Bundled/GallopIcons.xcassets from the Gallop SVGs in Bundled/Icons/svg.

UIKit cannot read a bare SVG file, so iOS loads the icons from this asset catalog, which the package ships.
The Mac reads the SVG files directly. Each SVG becomes an imageset that keeps its vector data, named after
the file, so GallopIcon's raw values load with UIImage(named:in:). Rerun this after adding, changing or
removing an SVG; with nothing changed it changes nothing. A package test fails when the two drift apart.
"""

import json
import shutil
import sys
from pathlib import Path

BUNDLED = Path(__file__).resolve().parent.parent / "Sources" / "CowchatKit" / "Bundled"
SVG_DIR = BUNDLED / "Icons" / "svg"
CATALOG = BUNDLED / "GallopIcons.xcassets"
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
            "properties": {"preserves-vector-representation": True},
        })

    print(f"{len(svgs)} icons in {CATALOG.relative_to(BUNDLED.parent.parent.parent)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
