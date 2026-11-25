: "${MSVC_SDK:?set MSVC_SDK to the SDK directory created by ci/msvc/fetch-sdk.sh}"
MSVC_SDK=$(cd "$MSVC_SDK" && pwd)
export MSVC_SDK

CARGO_TARGET_I686_PC_WINDOWS_MSVC_LINKER=$(command -v lld-link || ls /usr/lib/llvm-*/bin/lld-link 2>/dev/null | sort -V | tail -n 1)
export CARGO_TARGET_I686_PC_WINDOWS_MSVC_LINKER
export CARGO_TARGET_I686_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static -Lnative=$MSVC_SDK/crt/lib/x86 -Lnative=$MSVC_SDK/sdk/lib/um/x86 -Lnative=$MSVC_SDK/sdk/lib/ucrt/x86"
