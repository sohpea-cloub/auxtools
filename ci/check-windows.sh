#!/bin/sh
set -eu
LIBRARIES="kernel32.dll ntdll.dll advapi32.dll ws2_32.dll userenv.dll bcrypt.dll bcryptprimitives.dll api-ms-win-core-synch-l1-2-0.dll"

readobj=$(command -v llvm-readobj || ls /usr/lib/llvm-*/bin/llvm-readobj 2>/dev/null | sort -V | tail -n 1)
[ -n "$readobj" ] || { echo "llvm-readobj isn't installed (it's in the llvm package)" >&2; exit 2; }

status=0
for file; do
	headers=$("$readobj" --file-headers "$file")
	echo "$headers" | grep -q 'Machine: IMAGE_FILE_MACHINE_I386' || { echo "$file isn't a 32-bit x86 program"; status=1; }
	needs=$(echo "$headers" | sed -n 's/.*MajorSubsystemVersion: //p')
	[ "$needs" -le 10 ] || { echo "$file asks for Windows $needs"; status=1; }
	libraries=$("$readobj" --coff-imports "$file" | sed -n 's/^  Name: //p' | tr 'A-Z' 'a-z' | sort -u | tr '\n' ' ')
	echo "$file: $libraries"
	for library in $libraries; do
		case " $LIBRARIES " in
			*" $library "*) ;;
			*) echo "$file imports unexpected DLL: $library (verify Windows 10 availability before updating $0)"; status=1 ;;
		esac
	done
done
exit $status
