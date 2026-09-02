#!/usr/bin/env bash
#
# Bootstrap script — builds the four SDL2 libraries (core, mixer, image, ttf)
# from source into third_party/sdl2/ so the game builds and runs without any
# host-installed SDL. Rust-only house rule means this is the one place we
# touch C tooling (cmake + a C compiler), and only for building the deps.
#
# Why this exists:
#   * Homebrew on macOS now ships `sdl2-compat` (an SDL3 shim) instead of real
#     SDL2. Linking rust-sdl2 against it panics on the event parser
#     ("invalid value 0x207").
#   * rust-sdl2's `bundled` feature compiles ONLY SDL2 core — it explicitly
#     does not handle SDL2_mixer/SDL2_image/SDL2_ttf. Those must come from
#     somewhere else on every platform.
#   * Bundling all four from source gives us identical, portable deps with no
#     reliance on any OS package manager.
#
# Usage:
#   ./scripts/bootstrap-sdl2.sh          # build into third_party/sdl2/
#
# Idempotent: skips building a component whose libs already exist, so it's
# cheap to re-run and safe to call from CI or a fresh clone.

set -euo pipefail

cd "$(dirname "$0")/.."           # project root
INSTALL_PREFIX="$(pwd)/third_party/sdl2"
mkdir -p "$INSTALL_PREFIX"

# Pinned, known-good releases (SDL2 era, not SDL3).
SDL2_VER="2.30.12"
MIXER_VER="2.8.2"
IMAGE_VER="2.8.5"
TTF_VER="2.24.0"

DL_ROOT="$(mktemp -d)"
trap 'rm -rf "$DL_ROOT"' EXIT

command -v cmake >/dev/null || { echo "error: cmake not found (needed to build SDL2)"; exit 1; }
command -v curl >/dev/null || { echo "error: curl not found"; exit 1; }

# Number of parallel build jobs (macOS/BSD vs Linux flag).
JOBS="${JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"

build_component() {
    local name="$1" url="$2" srcver="$3" prefix_flag="$4" prefix_flag_val="$5"
    shift 5
    local extra_cmake=("$@")

    # Skip if already built for this prefix. Match on the installed library
    # names rather than the source version, because the resulting dylib name
    # encodes the SDL ABI (e.g. libSDL2_mixer-2.0.0.dylib), not the tarball
    # version. A directory with existing .lib/.dylib/.a files counts as built.
    if [ -d "$INSTALL_PREFIX/lib" ] && ls "$INSTALL_PREFIX"/lib/ 2>/dev/null | grep -qi "^lib${name}[.-]"; then
        echo "== $name: already built, skipping =="
        return 0
    fi

    echo "== $name: downloading =="
    curl -fsSL -o "$DL_ROOT/$name.tar.gz" "$url"
    tar -xzf "$DL_ROOT/$name.tar.gz" -C "$DL_ROOT"

    local src_dir="$DL_ROOT/$(basename "$name")-$srcver"
    local build_dir="$src_dir/build"
    mkdir -p "$build_dir"
    pushd "$build_dir" >/dev/null

    echo "== $name: configuring =="
    cmake .. \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
        -DCMAKE_INSTALL_PREFIX="$INSTALL_PREFIX" \
        -DCMAKE_OSX_ARCHITECTURES="${CMAKE_OSX_ARCHITECTURES:-$(uname -m)}" \
        -DCMAKE_PREFIX_PATH="$INSTALL_PREFIX" \
        "$prefix_flag=$prefix_flag_val" \
        "${extra_cmake[@]}" \
        "$@"

    echo "== $name: building =="
    cmake --build . -j "$JOBS"

    echo "== $name: installing =="
    cmake --install . >/dev/null

    popd >/dev/null
}

build_component "SDL2" \
    "https://github.com/libsdl-org/SDL/releases/download/release-$SDL2_VER/SDL2-$SDL2_VER.tar.gz" \
    "$SDL2_VER" \
    "-DCMAKE_INSTALL_PREFIX" "$INSTALL_PREFIX" \
    -DSDL_SHARED=ON -DSDL_STATIC=OFF

# SDL2_mixer: disable Opus (its Homebrew libopusfile header mismatches the
# vendored one and breaks the build).
build_component "SDL2_mixer" \
    "https://github.com/libsdl-org/SDL_mixer/releases/download/release-$MIXER_VER/SDL2_mixer-$MIXER_VER.tar.gz" \
    "$MIXER_VER" \
    "-DCMAKE_INSTALL_PREFIX" "$INSTALL_PREFIX" \
    -DCMAKE_DISABLE_FIND_PACKAGE_OpusFile=TRUE

build_component "SDL2_image" \
    "https://github.com/libsdl-org/SDL_image/releases/download/release-$IMAGE_VER/SDL2_image-$IMAGE_VER.tar.gz" \
    "$IMAGE_VER" \
    "-DCMAKE_INSTALL_PREFIX" "$INSTALL_PREFIX"

build_component "SDL2_ttf" \
    "https://github.com/libsdl-org/SDL_ttf/releases/download/release-$TTF_VER/SDL2_ttf-$TTF_VER.tar.gz" \
    "$TTF_VER" \
    "-DCMAKE_INSTALL_PREFIX" "$INSTALL_PREFIX"

# Rewrite the *.pc files to point at our install prefix. cmake writes the
# absolute prefix it was configured with, which matches INSTALL_PREFIX here —
# but we re-write defensively so the tree stays relocatable if someone moves it.
for pc in "$INSTALL_PREFIX"/lib/pkgconfig/*.pc; do
    sed -i.bak "s|^prefix=.*|prefix=$INSTALL_PREFIX|" "$pc" && rm -f "$pc.bak"
done

echo ""
echo "SDL2 stack installed to: $INSTALL_PREFIX"
echo "Now run: cargo build --workspace"
