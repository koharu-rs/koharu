#!/usr/bin/env python3
"""Fetch the pinned prebuilt runtime and generate resources from their owners."""
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import urllib.request

APP = Path(__file__).resolve().parents[1]
REPO = APP.parents[1]
VERSION = "b12024"
SHA256 = "b7005ad25dab07d273794afc7ac24d55aeac1b52376301907c5fb4762e42545d"
ARCHIVE = APP / "Vendor" / f"llama-{VERSION}-xcframework.zip"
ARCHIVE.parent.mkdir(exist_ok=True)
if not ARCHIVE.exists():
    url = f"https://github.com/ggml-org/llama.cpp/releases/download/{VERSION}/{ARCHIVE.name}"
    temporary = ARCHIVE.with_suffix(".download")
    try:
        urllib.request.urlretrieve(url, temporary)
        temporary.replace(ARCHIVE)
    finally:
        temporary.unlink(missing_ok=True)
checksum = hashlib.sha256()
with ARCHIVE.open("rb") as archive:
    for chunk in iter(lambda: archive.read(1024 * 1024), b""):
        checksum.update(chunk)
if checksum.hexdigest() != SHA256:
    raise SystemExit(f"Runtime checksum mismatch: {ARCHIVE}. Remove it and run again.")
# ditto preserves framework symlinks; zipfile.extractall does not.
subprocess.run(["ditto", "-x", "-k", str(ARCHIVE), str(ARCHIVE.parent)], check=True)

# Official prebuilt stable-diffusion.cpp wrapper: includes Metal and iOS slices.
SD_VERSION = "v0.3.0"
SD_SHA256 = "3b83a0c21a00f6418890b679ea45b00c20d012da0a57fe989635b793e09f096a"
sd_archive = APP / "Vendor" / f"stable-diffusion-native-apple-xcframework-{SD_VERSION}.zip"
if not sd_archive.exists():
    temporary = sd_archive.with_suffix(".download")
    try:
        urllib.request.urlretrieve(f"https://github.com/leehack/stable-diffusion-native/releases/download/{SD_VERSION}/{sd_archive.name}", temporary)
        temporary.replace(sd_archive)
    finally:
        temporary.unlink(missing_ok=True)
checksum = hashlib.sha256()
with sd_archive.open("rb") as archive:
    for chunk in iter(lambda: archive.read(1024 * 1024), b""):
        checksum.update(chunk)
if checksum.hexdigest() != SD_SHA256:
    raise SystemExit(f"Diffusion runtime checksum mismatch: {sd_archive}")
subprocess.run(["ditto", "-x", "-k", str(sd_archive), str(APP / "Vendor/sd-native")], check=True)

language_source = (REPO / "crates/koharu-translator/src/language.rs").read_text()
languages = re.findall(r'to_string\s*=\s*"([^"]+)"(?:(?!to_string).)*?props\(tag\s*=\s*"([^"]+)"\)', language_source, re.S)
if not languages:
    raise SystemExit("Could not read the authoritative translation language catalog")
(APP / "Resources/languages.json").write_text(json.dumps([
    {"tag": tag, "name": name} for name, tag in languages
], indent=2, ensure_ascii=False) + "\n")
with tempfile.TemporaryDirectory() as directory:
    icons = Path(directory) / "koharu.iconset"
    subprocess.run(["iconutil", "--convert", "iconset", str(REPO / "crates/koharu/icons/icon.icns"), "--output", str(icons)], check=True)
    shutil.copyfile(icons / "icon_512x512@2x.png", APP / "Resources/Assets.xcassets/AppIcon.appiconset/Icon.png")
subprocess.run(["xcodegen", "generate", "--spec", "project.yml"], cwd=APP, check=True)
print(f"Generated project; {len(languages)} target languages; llama.cpp {VERSION} and diffusion {SD_VERSION} verified.")
