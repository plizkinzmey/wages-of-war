# Bootstrap script (Windows / PowerShell) — builds the four SDL2 libraries
# (core, mixer, image, ttf) from source into third_party/sdl2/.
#
# Mirrors scripts/bootstrap-sdl2.sh. Rust-only house rule means this is the one
# place we touch C tooling (cmake + a C compiler), and only to build deps.
#
# Why this exists:
#   * rust-sdl2's `bundled` feature compiles ONLY SDL2 core — it does not build
#     SDL2_mixer/SDL2_image/SDL2_ttf, which this project needs.
#   * Vendoring all four from source gives identical, portable deps with no
#     reliance on any OS package manager.
#   * .cargo/config.toml points PKG_CONFIG_PATH / LIBRARY_PATH at the result.
#
# Usage:
#   .\scripts\bootstrap-sdl2.ps1
#
# Idempotent: skips building a component whose libs already exist.

param(
    [string]$Arch = "x64"
)

$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
$InstallPrefix = (Resolve-Path ".").Path + "\third_party\sdl2"
New-Item -ItemType Directory -Force -Path $InstallPrefix | Out-Null

# Pinned, known-good releases (SDL2 era, not SDL3).
$SDL2Ver = "2.30.12"
$MixerVer = "2.8.2"
$ImageVer = "2.8.5"
$TtfVer  = "2.24.0"

# Locate cmake.
$cmake = Get-Command cmake -ErrorAction SilentlyContinue
if (-not $cmake) { Write-Error "cmake not found (needed to build SDL2). Install via: winget install Kitware.CMake"; exit 1 }

$tmp = Join-Path $env:TEMP "ow-sdl2-bootstrap"
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

function Build-Component {
    param(
        [string]$Name,
        [string]$Version,
        [string]$Url
    )

    # Skip if already built for this prefix (any matching lib file).
    if (Test-Path (Join-Path $InstallPrefix "lib")) {
        $existing = Get-ChildItem (Join-Path $InstallPrefix "lib") -Filter "*$Version*" -ErrorAction SilentlyContinue
        if ($existing) { Write-Host "== $Name: already built, skipping =="; return }
    }

    Write-Host "== $Name: downloading =="
    $archive = Join-Path $tmp "$Name.tar.gz"
    Invoke-WebRequest -Uri $Url -OutFile $archive

    Write-Host "== $Name: extracting =="
    tar -xzf $archive -C $tmp

    $srcDir = Join-Path $tmp "$Name-$Version"
    $buildDir = Join-Path $srcDir "build"
    New-Item -ItemType Directory -Force -Path $buildDir | Out-Null

    Write-Host "== $Name: configuring =="
    & $cmake.Source .. `
        "-DCMAKE_POLICY_VERSION_MINIMUM=3.5" `
        "-DCMAKE_INSTALL_PREFIX=$InstallPrefix" `
        "-DCMAKE_PREFIX_PATH=$InstallPrefix" `
        "-DCMAKE_GENERATOR_PLATFORM=$Arch" `
        @args
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "== $Name: building =="
    & cmake --build . --config Release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "== $Name: installing =="
    & cmake --install . --config Release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

Push-Location (Join-Path $tmp "obj") # placeholder; real work happens in cmake build dirs

Build-Component "SDL2" $SDL2Ver `
    "https://github.com/libsdl-org/SDL/releases/download/release-$SDL2Ver/SDL2-$SDL2Ver.tar.gz" `
    "-DSDL_SHARED=ON" "-DSDL_STATIC=OFF"

# SDL2_mixer: disable Opus to avoid header mismatches with vendored libs.
Build-Component "SDL2_mixer" $MixerVer `
    "https://github.com/libsdl-org/SDL_mixer/releases/download/release-$MixerVer/SDL2_mixer-$MixerVer.tar.gz" `
    "-DCMAKE_DISABLE_FIND_PACKAGE_OpusFile=TRUE"

Build-Component "SDL2_image" $ImageVer `
    "https://github.com/libsdl-org/SDL_image/releases/download/release-$ImageVer/SDL2_image-$ImageVer.tar.gz"

Build-Component "SDL2_ttf" $TtfVer `
    "https://github.com/libsdl-org/SDL_ttf/releases/download/release-$TtfVer/SDL2_ttf-$TtfVer.tar.gz"

Pop-Location

Write-Host ""
Write-Host "SDL2 stack installed to: $InstallPrefix"
Write-Host "Now run: cargo build --workspace"
