#!/usr/bin/env bash
# Builds the app's native libraries into app/src/main/jniLibs/<abi>/, for the phone (arm64-v8a)
# and the emulator (x86_64):
#   libutter_mobile.so   the Rust core (crates/utter-mobile)
#   libllama-server.so   llama.cpp's server at the desktop's pinned build, CPU backend; an
#                        executable named as a library, the only way Android runs one
# onnxruntime and LiteRT come from their Maven packages (app/build.gradle.kts).
#
# Needs the Android NDK (ANDROID_NDK_HOME, else the SDK's ndk/29.0.14206865), the SDK's CMake,
# and rustup's aarch64-linux-android and x86_64-linux-android targets.
#
#   android/native/build.sh [arm64-v8a|x86_64]...   (default: both)
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
SDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/AppData/Local/Android/Sdk}}
NDK=${ANDROID_NDK_HOME:-$SDK/ndk/29.0.14206865}
API=34
LLAMA_TAG=b11535
LLAMA_SHA256=0699a2905220889d72ce1bf269a60611598c584180754d21083f935c485c945c
WORK=${UTTER_NATIVE_WORK:-$REPO/target/android-native}
OUT=$REPO/android/app/src/main/jniLibs

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) HOST=windows-x86_64; EXE=.exe; CMD=.cmd ;;
  Darwin) HOST=darwin-x86_64; EXE=; CMD= ;;
  *) HOST=linux-x86_64; EXE=; CMD= ;;
esac
BIN=$NDK/toolchains/llvm/prebuilt/$HOST/bin
CMAKE_DIR=$(ls -d "$SDK"/cmake/* | sort -V | tail -1)/bin

llama_source() {
  local src=$WORK/llama.cpp-$LLAMA_TAG
  if [ ! -d "$src" ]; then
    mkdir -p "$WORK"
    local tar=$WORK/llama.cpp-$LLAMA_TAG.tar.gz
    curl -fsSL -o "$tar" "https://github.com/ggml-org/llama.cpp/archive/refs/tags/$LLAMA_TAG.tar.gz"
    echo "$LLAMA_SHA256  $tar" | sha256sum -c - >/dev/null
    tar -xzf "$tar" -C "$WORK"
  fi
  echo "$src"
}

build_abi() {
  local abi=$1 target cpu_flags
  case "$abi" in
    arm64-v8a) target=aarch64-linux-android; cpu_flags="-DGGML_CPU_ARM_ARCH=armv9.2-a+i8mm+dotprod+bf16" ;;
    x86_64) target=x86_64-linux-android; cpu_flags="-DGGML_NATIVE=OFF" ;;
    *) echo "unknown ABI $abi" >&2; exit 2 ;;
  esac
  mkdir -p "$OUT/$abi"

  # The Rust core.
  local triple_env=${target//-/_}
  local clang=$BIN/$target$API-clang$CMD
  env "CARGO_TARGET_${triple_env^^}_LINKER=$clang" "CC_$triple_env=$clang" "AR_$triple_env=$BIN/llvm-ar$EXE" \
    cargo build --manifest-path "$REPO/Cargo.toml" --release --target "$target" -p utter-mobile
  "$BIN/llvm-strip$EXE" -o "$OUT/$abi/libutter_mobile.so" "$REPO/target/$target/release/libutter_mobile.so"

  # llama-server, as the phone benchmarks ran it (docs/plan-android.md).
  local src build
  src=$(llama_source)
  build=$WORK/llama-$abi
  "$CMAKE_DIR/cmake$EXE" -S "$src" -B "$build" -G Ninja \
    -DCMAKE_MAKE_PROGRAM="$CMAKE_DIR/ninja$EXE" \
    -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
    -DANDROID_ABI="$abi" -DANDROID_PLATFORM="android-$API" -DCMAKE_BUILD_TYPE=Release \
    -DGGML_OPENMP=OFF $cpu_flags \
    -DLLAMA_CURL=OFF -DLLAMA_OPENSSL=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF \
    -DBUILD_SHARED_LIBS=OFF >/dev/null
  "$CMAKE_DIR/cmake$EXE" --build "$build" --target llama-server -j "$(nproc)" >/dev/null
  "$BIN/llvm-strip$EXE" -o "$OUT/$abi/libllama-server.so" "$build/bin/llama-server"
  echo "$abi: $(ls "$OUT/$abi" | tr '\n' ' ')"
}

ABIS=("$@")
[ ${#ABIS[@]} -eq 0 ] && ABIS=(arm64-v8a x86_64)
for abi in "${ABIS[@]}"; do build_abi "$abi"; done
