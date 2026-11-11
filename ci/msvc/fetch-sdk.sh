#!/bin/sh
# Fetches what building for i686-pc-windows-msvc needs of Visual Studio, on
# Linux: Microsoft's C runtime and the Windows SDK's headers and import
# libraries.
#
#     ci/msvc/fetch-sdk.sh DIRECTORY
#
# xwin (https://github.com/Jake-Shadle/xwin) downloads them from Microsoft and
# lays them out in DIRECTORY, about 800 MB. Running this accepts Microsoft's
# licence for them: https://go.microsoft.com/fwlink/?LinkId=2086102
#
# The versions are fixed here so every build links the same libraries. Programs
# linked with them run on Windows 10 and later: the C runtime is linked into
# them (see env.sh), and the SDK only decides which system DLLs they can name.
set -eu
[ $# -eq 1 ] || { sed -n '2,8p' "$0" >&2; exit 2; }
dest=$1

XWIN_VERSION=0.10.0
XWIN_SHA256=d870eb4b2f390878af6da1ccd3cf321d22fcb72720984853b4be732ae597fc88
SDK_VERSION=10.0.26100
CRT_VERSION=14.44.17.14

stamp="xwin $XWIN_VERSION, SDK $SDK_VERSION, CRT $CRT_VERSION"
if [ "$(cat "$dest/version" 2>/dev/null)" = "$stamp" ]; then
	echo "$dest has $stamp already"
	exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fsSL --retry 3 -o "$work/xwin.tar.gz" \
	"https://github.com/Jake-Shadle/xwin/releases/download/$XWIN_VERSION/xwin-$XWIN_VERSION-x86_64-unknown-linux-musl.tar.gz"
echo "$XWIN_SHA256  $work/xwin.tar.gz" | sha256sum -c -
tar -xzf "$work/xwin.tar.gz" -C "$work" --strip-components=1

rm -rf "$dest"
"$work/xwin" --accept-license --arch x86 --sdk-version "$SDK_VERSION" --crt-version "$CRT_VERSION" \
	--cache-dir "$work/cache" splat --output "$dest"
echo "$stamp" >"$dest/version"
