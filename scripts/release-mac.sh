#!/bin/bash
# Builds the macOS release on this Mac and attaches it to the GitHub release of the
# current version: Corolla-<version>-macos-arm64.dmg and
# Corolla-<version>-macos-extras.zip (the double-click installer for the free tools).
# The Windows installer is built by CI when the tag is pushed; whichever finishes first
# creates the release, the other adds its files.
#
# Usage, from a clean checkout of the tagged commit:
#   git push origin vX.Y.Z && bash scripts/release-mac.sh
# Needs: gh (logged in), Node.js, Rust, CMake, Xcode command line tools.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

fail() { echo "release-mac: $*" >&2; exit 1; }

version="$(python3 -c "import json;print(json.load(open('package.json'))['version'])")"
tag="v$version"
[ "$(uname -m)" = "arm64" ] || fail "build on an Apple silicon Mac"
git diff --quiet && git diff --cached --quiet || fail "commit or stash your changes first"
git rev-parse -q --verify "refs/tags/$tag" > /dev/null || fail "tag $tag doesn't exist"
[ "$(git rev-parse HEAD)" = "$(git rev-parse "$tag^{commit}")" ] || fail "HEAD isn't $tag; check it out first"
git ls-remote --exit-code --tags origin "refs/tags/$tag" > /dev/null || fail "push the tag first: git push origin $tag"

echo "== Engines"
bash scripts/fetch-engines.sh

echo "== Tests"
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
# Only what the app carries, as a Mac without any extras has it.
COROLLA_TEST_WITHOUT=ffmpeg,magick,fontforge \
  cargo test --manifest-path src-tauri/Cargo.toml every_pair -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml abuse_ -- --ignored

echo "== Build"
npm ci
npx tauri build
dmg="$(ls src-tauri/target/release/bundle/dmg/Corolla_"${version}"_aarch64.dmg)"
python3 tools/check_package.py "$dmg"

rm -rf dist && mkdir dist
cp "$dmg" "dist/Corolla-$version-macos-arm64.dmg"
# zip keeps the file's executable bit, which a plain download would lose.
(cd macos && zip -X -q "../dist/Corolla-$version-macos-extras.zip" "Install Corolla Extras.command")

echo "== Upload to the $tag release"
git tag -l --format='%(contents)' "$tag" > dist/notes.md
[ -s dist/notes.md ] || echo "Corolla $tag" > dist/notes.md
if ! gh release view "$tag" > /dev/null 2>&1; then
  gh release create "$tag" --verify-tag --title "Corolla $tag" --notes-file dist/notes.md \
    || gh release view "$tag" > /dev/null  # CI may have created it meanwhile
fi
gh release upload "$tag" "dist/Corolla-$version-macos-arm64.dmg" "dist/Corolla-$version-macos-extras.zip" --clobber
gh release view "$tag" --json assets --jq '.assets[] | "\(.name)  \(.size) bytes"'
