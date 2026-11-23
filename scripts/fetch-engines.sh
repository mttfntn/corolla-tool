#!/bin/bash
# Fills src-tauri/binaries/macos with the conversion engines. Safe to re-run.
# Needs: Xcode command line tools and cmake (brew install cmake).
# Images use macOS's built-in `sips` plus FFmpeg, so no ImageMagick is needed.
# Cache folders end in -mac: build-cache is shared with fetch-engines.ps1, which
# empties its own folders (7zip, pdfium, assimp) before unpacking into them.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/src-tauri/binaries/macos"
cache="$root/build-cache"
mkdir -p "$out" "$cache"

arch="$(uname -m)"   # arm64 or x86_64
[ "$arch" = "arm64" ] && pdfium_arch="arm64" || pdfium_arch="x64"

# GitHub's API allows few anonymous requests per address, which shared CI machines
# use up; CI passes its token in GITHUB_TOKEN. Without it, requests stay anonymous.
api() { # url
  if [ -n "${GITHUB_TOKEN:-}" ]; then
    curl -fsSL --retry 3 -H "Authorization: Bearer $GITHUB_TOKEN" "$1"
  else
    curl -fsSL --retry 3 "$1"
  fi
}

latest_asset() { # repo, regex
  api "https://api.github.com/repos/$1/releases/latest" \
    | grep -o '"browser_download_url": *"[^"]*"' | cut -d'"' -f4 | grep -E "$2" | head -1
}

if [ ! -x "$out/7zz" ]; then
  url="$(latest_asset ip7z/7zip 'mac\.tar\.xz$')"
  curl -fL --retry 3 -o "$cache/7zip-mac.tar.xz" "$url"
  rm -rf "$cache/7zip-mac" && mkdir -p "$cache/7zip-mac"
  tar -xf "$cache/7zip-mac.tar.xz" -C "$cache/7zip-mac"
  cp "$cache/7zip-mac/7zz" "$out/7zz"
fi

if [ ! -f "$out/libpdfium.dylib" ]; then
  url="$(latest_asset bblanchon/pdfium-binaries "^.*/pdfium-mac-$pdfium_arch\.tgz$")"
  curl -fL --retry 3 -o "$cache/pdfium-mac.tgz" "$url"
  rm -rf "$cache/pdfium-mac" && mkdir -p "$cache/pdfium-mac"
  tar -xzf "$cache/pdfium-mac.tgz" -C "$cache/pdfium-mac"
  cp "$cache/pdfium-mac/lib/libpdfium.dylib" "$out/"
fi

if [ ! -x "$out/ffmpeg" ]; then
  # Not bundled: the app would need a self-contained build, and Homebrew's depends on
  # other libraries. Corolla finds an installed FFmpeg (also Homebrew's, from the Finder).
  echo "FFmpeg: not bundled. Install it with 'brew install ffmpeg' for audio, video and"
  echo "more image formats, or put self-contained ffmpeg and ffprobe for $arch in $out."
  if command -v ffmpeg >/dev/null; then
    echo "An installed ffmpeg was found at $(command -v ffmpeg); Corolla will use it."
  fi
fi

if [ ! -x "$out/assimp" ]; then
  tag="$(api https://api.github.com/repos/assimp/assimp/releases/latest | grep -o '"tag_name": *"[^"]*"' | cut -d'"' -f4)"
  curl -fL --retry 3 -o "$cache/assimp-mac.tar.gz" "https://github.com/assimp/assimp/archive/refs/tags/$tag.tar.gz"
  rm -rf "$cache/assimp-mac" && mkdir -p "$cache/assimp-mac"
  tar -xzf "$cache/assimp-mac.tar.gz" -C "$cache/assimp-mac" --strip-components=1
  # macOS 13 like the bundled PDFium (and the app's minimum); CMake would
  # otherwise target only the macOS version doing the build.
  cmake -S "$cache/assimp-mac" -B "$cache/assimp-mac/build" -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=13.0 \
    -DBUILD_SHARED_LIBS=OFF -DASSIMP_BUILD_ASSIMP_TOOLS=ON -DASSIMP_BUILD_TESTS=OFF \
    -DASSIMP_BUILD_SAMPLES=OFF -DASSIMP_WARNINGS_AS_ERRORS=OFF -DASSIMP_INSTALL=OFF
  cmake --build "$cache/assimp-mac/build" --target assimp_cmd --parallel
  cp "$(find "$cache/assimp-mac/build" -type f -name assimp -perm -u+x | head -1)" "$out/assimp"
fi

# STEP files: Mayo has no macOS release, so they need FreeCAD installed on this Mac.
if [ ! -x /Applications/FreeCAD.app/Contents/Resources/bin/freecadcmd ] && [ ! -x /Applications/FreeCAD.app/Contents/MacOS/FreeCADCmd ]; then
  echo "STEP: install FreeCAD to convert STEP files on this Mac; otherwise those circles stay hidden."
fi

# Fonts: TTF <-> OTF needs FontForge installed on this Mac; WOFF and WOFF2 work without it.
if [ ! -d /Applications/FontForge.app ] && ! command -v fontforge >/dev/null; then
  echo "Fonts: install FontForge (fontforge.org) to convert between TTF and OTF."
fi

# License texts, shipped beside the engines (THIRD-PARTY-NOTICES.md lists them all).
mkdir -p "$out/licenses"
cp "$root/THIRD-PARTY-NOTICES.md" "$root/LICENSE" "$out/licenses/"
copy_license() { # from, name: from the download when it's there, else keep last time's copy
  if [ -f "$1" ]; then cp "$1" "$out/licenses/$2"
  elif [ ! -f "$out/licenses/$2" ]; then
    echo "License missing: $1. Delete the engine from $out and re-run to fetch it again."; exit 1
  fi
}
copy_license "$cache/7zip-mac/License.txt" 7-Zip.txt
copy_license "$cache/pdfium-mac/LICENSE" PDFium.txt
if [ -d "$cache/pdfium-mac/licenses" ]; then
  rm -rf "$out/licenses/PDFium-third-party" && cp -R "$cache/pdfium-mac/licenses" "$out/licenses/PDFium-third-party"
elif [ ! -d "$out/licenses/PDFium-third-party" ]; then
  echo "PDFium's third-party licenses are missing in $cache/pdfium-mac."; exit 1
fi
copy_license "$cache/assimp-mac/LICENSE" Assimp.txt

touch "$out/.keep"
echo "Engines in $out:"
ls -la "$out"
