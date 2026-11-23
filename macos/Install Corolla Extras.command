#!/bin/bash
# Double-click to install the free tools Corolla uses on a Mac, with Homebrew:
# FFmpeg (audio, video, more pictures), ImageMagick (ICO, WebP and AVIF output,
# better compression), FontForge (TTF <-> OTF), FreeCAD (STEP files) and rar
# (making RAR archives). LibreOffice and Blender only improve two conversions and
# are large, so they're asked about. Homebrew itself is installed first if it's
# missing; it asks for your Mac password once.
#
# Safe to run again: Homebrew skips what's already installed.
# DRY_RUN=1 prints the commands instead of running them.

set -u

run() {
  if [ "${DRY_RUN:-}" = "1" ]; then echo "would run: $*"; else "$@"; fi
}

ask() { # question; true for yes (the default is no)
  local answer
  read -r -p "$1 [y/N] " answer
  [[ "$answer" =~ ^[Yy] ]]
}

finish() {
  echo
  read -r -p "Press Return to close this window." _
  exit "$1"
}

echo "Corolla extras for macOS"
echo "========================"
echo

if [ "$(uname -m)" != "arm64" ]; then
  echo "This Mac has an Intel processor. Corolla for macOS needs Apple silicon (M1 or newer)."
  finish 1
fi

# Homebrew lives in /opt/homebrew on Apple silicon; a double-clicked script doesn't
# get the shell's PATH, so look there directly.
BREW=""
for candidate in /opt/homebrew/bin/brew /usr/local/bin/brew "$(command -v brew 2>/dev/null)"; do
  if [ -n "$candidate" ] && [ -x "$candidate" ]; then BREW="$candidate"; break; fi
done

if [ -z "$BREW" ]; then
  echo "Homebrew isn't installed. It's the usual way to install free tools on a Mac"
  echo "(https://brew.sh). Its installer will ask for your Mac password and may"
  echo "install Apple's command line tools first, which takes a few minutes."
  echo
  if ! ask "Install Homebrew now?"; then
    echo "Nothing was installed."
    finish 1
  fi
  run /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  BREW=/opt/homebrew/bin/brew
  if [ "${DRY_RUN:-}" != "1" ] && [ ! -x "$BREW" ]; then
    echo "Homebrew didn't install. Nothing else was changed."
    finish 1
  fi
fi
echo "Using Homebrew at $BREW"
echo

echo "Installing FFmpeg, ImageMagick and FontForge..."
run "$BREW" install ffmpeg imagemagick fontforge || {
  echo "That didn't finish. Check your internet connection and run this file again."
  finish 1
}

echo
echo "Installing FreeCAD (STEP files, about 700 MB) and rar (RAR archives)..."
run "$BREW" install --cask freecad rar || echo "FreeCAD or rar didn't install; the rest still works."

echo
echo "Two optional extras, about 1 GB each:"
echo "  LibreOffice: PDF to Word keeps the layout instead of plain text."
echo "  Blender: FBX conversions keep rigs and animation better."
if ask "Install LibreOffice?"; then
  run "$BREW" install --cask libreoffice || echo "LibreOffice didn't install."
fi
if ask "Install Blender?"; then
  run "$BREW" install --cask blender || echo "Blender didn't install."
fi

# Corolla looks for tools when it starts, so restart it if it's running.
if pgrep -x corolla > /dev/null; then
  echo
  echo "Restarting Corolla so it finds the new tools..."
  run osascript -e 'quit app "Corolla"'
  sleep 2
  run open -a Corolla
fi

echo
echo "Done. Corolla now offers the extra formats."
finish 0
