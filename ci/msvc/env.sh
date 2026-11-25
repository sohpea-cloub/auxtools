: "${MSVC_SDK:?set MSVC_SDK to the SDK directory created by ci/msvc/fetch-sdk.sh}"
MSVC_SDK=$(cd "$MSVC_SDK" && pwd)
export MSVC_SDK

msvc_llvm_tool() {
	command -v "$1" || ls /usr/lib/llvm-*/bin/"$1" 2>/dev/null | sort -V | tail -n 1
}

MSVC_CLANG=$(msvc_llvm_tool clang)
export MSVC_CLANG
PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/bin:$PATH"
export PATH
export CC_i686_pc_windows_msvc=clang-cl
export CXX_i686_pc_windows_msvc=clang-cl
AR_i686_pc_windows_msvc=$(msvc_llvm_tool llvm-lib)
export AR_i686_pc_windows_msvc
CARGO_TARGET_I686_PC_WINDOWS_MSVC_LINKER=$(msvc_llvm_tool lld-link)
export CARGO_TARGET_I686_PC_WINDOWS_MSVC_LINKER
export CARGO_TARGET_I686_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/PDBALTPATH:%_PDB% -Lnative=$MSVC_SDK/crt/lib/x86 -Lnative=$MSVC_SDK/sdk/lib/um/x86 -Lnative=$MSVC_SDK/sdk/lib/ucrt/x86"
