# Source me, from bash, with MSVC_SDK set to the directory fetch-sdk.sh filled:
#
#     export MSVC_SDK=DIRECTORY
#     . ci/msvc/env.sh
#
# Has cargo build for i686-pc-windows-msvc on Linux, with LLVM's tools in place
# of Visual Studio's (clang, lld and llvm are the packages): clang compiles the
# C++, lld-link links, and the libraries are those in MSVC_SDK. Microsoft's C++
# headers want clang 19 or newer. Then:
#
#     cargo build --target i686-pc-windows-msvc
#
# This file's settings take the place of the target's in .cargo/config.toml.

: "${MSVC_SDK:?set MSVC_SDK to the directory ci/msvc/fetch-sdk.sh filled}"
MSVC_SDK=$(cd "$MSVC_SDK" && pwd)
export MSVC_SDK

# A tool of LLVM's by name, or the newest of those installed with a version
msvc_llvm_tool() {
	command -v "$1" || ls /usr/lib/llvm-*/bin/"$1" 2>/dev/null | sort -V | tail -n 1
}

MSVC_CLANG=$(msvc_llvm_tool clang)
export MSVC_CLANG
# bin/ has clang-cl, and an ml.exe for the assembly MASM would build
PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/bin:$PATH"
export PATH
export CC_i686_pc_windows_msvc=clang-cl
export CXX_i686_pc_windows_msvc=clang-cl
AR_i686_pc_windows_msvc=$(msvc_llvm_tool llvm-lib)
export AR_i686_pc_windows_msvc
CARGO_TARGET_I686_PC_WINDOWS_MSVC_LINKER=$(msvc_llvm_tool lld-link)
export CARGO_TARGET_I686_PC_WINDOWS_MSVC_LINKER
# The C runtime is linked in, so the programs need no Visual C++
# redistributable. A program names its .pdb by file name alone, not by where
# this machine built it.
export CARGO_TARGET_I686_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/PDBALTPATH:%_PDB% -Lnative=$MSVC_SDK/crt/lib/x86 -Lnative=$MSVC_SDK/sdk/lib/um/x86 -Lnative=$MSVC_SDK/sdk/lib/ucrt/x86"
