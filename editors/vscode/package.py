#!/usr/bin/env python3
"""Packages the extension as a .vsix, with the launcher and debug library for
each platform built so far.

    python3 editors/vscode/package.py [--profile release] [--out FILE]

Run from the repository's root after building them, for example
`cargo build --release -p auxtools-cli -p debug_server` with
`--target i686-pc-windows-msvc` and `--target i686-unknown-linux-gnu`. A platform
whose files aren't built is left out, with a warning.

A .vsix is a zip: the extension under `extension/`, a manifest and a list of
content types. This writes it directly, so there's nothing to install.
"""

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent

# The platform name VS Code reports (process.platform), its Rust target, and
# the files from target/TARGET/PROFILE that go in bin/PLATFORM
PLATFORMS = [
	("win32", "i686-pc-windows-msvc", ["auxtools.exe", "debug_server.dll"]),
	("linux", "i686-unknown-linux-gnu", ["auxtools", "libdebug_server.so"]),
]

EXTENSION_FILES = ["package.json", "extension.js", "README.md"]


def manifest(package):
	return f"""<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">
	<Metadata>
		<Identity Language="en-US" Id="{escape(package["name"])}" Version="{escape(package["version"])}" Publisher="{escape(package["publisher"])}" />
		<DisplayName>{escape(package["displayName"])}</DisplayName>
		<Description xml:space="preserve">{escape(package["description"])}</Description>
		<Tags>{escape(",".join(package.get("keywords", [])))}</Tags>
		<Categories>{escape(",".join(package.get("categories", [])))}</Categories>
		<GalleryFlags>Public</GalleryFlags>
		<Properties>
			<Property Id="Microsoft.VisualStudio.Code.Engine" Value="{escape(package["engines"]["vscode"])}" />
			<Property Id="Microsoft.VisualStudio.Code.ExtensionKind" Value="workspace" />
		</Properties>
	</Metadata>
	<Installation>
		<InstallationTarget Id="Microsoft.VisualStudio.Code" />
	</Installation>
	<Dependencies />
	<Assets>
		<Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />
		<Asset Type="Microsoft.VisualStudio.Services.Content.Details" Path="extension/README.md" Addressable="true" />
	</Assets>
</PackageManifest>
"""


def content_types(paths):
	defaults = {".json": "application/json", ".js": "application/javascript", ".md": "text/markdown", ".vsixmanifest": "text/xml"}
	lines = ['<?xml version="1.0" encoding="utf-8"?>', '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">']
	for extension, kind in defaults.items():
		lines.append(f'\t<Default Extension="{extension}" ContentType="{kind}" />')
	# Everything else, binaries mostly, by name
	for path in paths:
		if Path(path).suffix not in defaults:
			lines.append(f'\t<Override PartName="/{escape(path)}" ContentType="application/octet-stream" />')
	lines.append("</Types>")
	return "\n".join(lines) + "\n"


def stripped(path):
	"""An ELF file without its debug info (the release profile keeps it), when
	there's a `strip` to do it"""
	if shutil.which("strip") is None or path.suffix in (".exe", ".dll"):
		return path.read_bytes()
	with tempfile.TemporaryDirectory() as tmp:
		out = Path(tmp) / path.name
		subprocess.run(["strip", "--strip-debug", "-o", str(out), str(path)], check=True)
		return out.read_bytes()


def add(archive, name, data, executable=False):
	info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
	info.compress_type = zipfile.ZIP_DEFLATED
	info.external_attr = (0o100755 if executable else 0o100644) << 16
	archive.writestr(info, data)


def main():
	parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
	parser.add_argument("--profile", default="release", help="the cargo profile the binaries were built with")
	parser.add_argument("--out", help="the .vsix to write (NAME-VERSION.vsix here unless set)")
	args = parser.parse_args()

	package = json.loads((HERE / "package.json").read_text())
	out = Path(args.out) if args.out else Path(f'{package["name"]}-{package["version"]}.vsix')

	files = {}
	for name in EXTENSION_FILES:
		files[f"extension/{name}"] = ((HERE / name).read_bytes(), False)
	files["extension/LICENSE"] = ((HERE / "LICENSE").read_bytes(), False)

	platforms = 0
	for platform, target, names in PLATFORMS:
		built = [ROOT / "target" / target / args.profile / name for name in names]
		missing = [str(path) for path in built if not path.is_file()]
		if missing:
			print(f"warning: leaving out {platform}, which isn't built ({', '.join(missing)})", file=sys.stderr)
			continue
		for path in built:
			files[f"extension/bin/{platform}/{path.name}"] = (stripped(path), True)
		platforms += 1

	if platforms == 0:
		sys.exit("nothing is built for any platform")

	with zipfile.ZipFile(out, "w") as archive:
		add(archive, "extension.vsixmanifest", manifest(package))
		add(archive, "[Content_Types].xml", content_types(list(files) + ["extension.vsixmanifest"]))
		for name, (data, executable) in sorted(files.items()):
			add(archive, name, data, executable)
	print(f"wrote {out} ({platforms} platform{'s' if platforms != 1 else ''})")


if __name__ == "__main__":
	main()
