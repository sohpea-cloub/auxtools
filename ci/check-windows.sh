#!/bin/sh
# Checks that programs built for Windows run on Windows 10 with nothing
# installed: they're 32-bit, as BYOND is, and the only DLLs they name are ones
# Windows 10 has. The C runtime is linked into them (ci/msvc/env.sh), so a
# Visual C++ redistributable's DLL here means that went wrong.
#
#     ci/check-windows.sh FILE...
set -eu
# What the Rust standard library uses, on Windows 10 and later
LIBRARIES="kernel32.dll ntdll.dll advapi32.dll ws2_32.dll userenv.dll bcrypt.dll bcryptprimitives.dll api-ms-win-core-synch-l1-2-0.dll"

readobj=$(command -v llvm-readobj || ls /usr/lib/llvm-*/bin/llvm-readobj 2>/dev/null | sort -V | tail -n 1)
[ -n "$readobj" ] || { echo "llvm-readobj isn't installed (it's in the llvm package)" >&2; exit 2; }

status=0
for file; do
	headers=$("$readobj" --file-headers "$file")
	echo "$headers" | grep -q 'Machine: IMAGE_FILE_MACHINE_I386' || { echo "$file isn't a 32-bit x86 program"; status=1; }
	# The Windows version a program says it needs: lld-link says Vista (6.0)
	needs=$(echo "$headers" | sed -n 's/.*MajorSubsystemVersion: //p')
	[ "$needs" -le 10 ] || { echo "$file asks for Windows $needs"; status=1; }
	libraries=$("$readobj" --coff-imports "$file" | sed -n 's/^  Name: //p' | tr 'A-Z' 'a-z' | sort -u | tr '\n' ' ')
	echo "$file: $libraries"
	for library in $libraries; do
		case " $LIBRARIES " in
			*" $library "*) ;;
			*) echo "$file needs $library: add it to $0 if every Windows 10 has it"; status=1 ;;
		esac
	done
done
exit $status
