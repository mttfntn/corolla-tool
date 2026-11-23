# Fills src-tauri\binaries\windows with the conversion engines. Safe to re-run:
# anything already present is skipped. Needs VS Build Tools 2022 for Assimp.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root "src-tauri\binaries\windows"
$cache = Join-Path $root "build-cache"
New-Item -ItemType Directory -Force $out, $cache | Out-Null

function Get-File($url, $name) {
    $path = Join-Path $cache $name
    if (-not (Test-Path $path)) {
        Write-Host "Downloading $url"
        curl.exe -fL --retry 3 -o "$path.part" $url
        if ($LASTEXITCODE -ne 0) { throw "Download failed: $url" }
        Move-Item "$path.part" $path
    }
    $path
}

function Get-ReleaseAsset($repo, $pattern) {
    $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers @{ "User-Agent" = "corolla-build" }
    $asset = $release.assets | Where-Object { $_.name -match $pattern } | Select-Object -First 1
    if (-not $asset) { throw "No asset matching $pattern in $repo $($release.tag_name)" }
    Get-File $asset.browser_download_url $asset.name
}

function New-Clean($name) {
    $dir = Join-Path $cache $name
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory $dir
}

# 7-Zip first: it unpacks ImageMagick. The MSI is extracted, not installed.
if (-not (Test-Path "$out\7z.exe")) {
    $msi = Get-ReleaseAsset "ip7z/7zip" "-x64\.msi$"
    $dir = New-Clean "7zip"
    Start-Process msiexec.exe -Wait -ArgumentList "/a", "`"$msi`"", "/qn", "TARGETDIR=`"$dir`""
    Copy-Item (Get-ChildItem $dir -Recurse -Filter 7z.exe | Select-Object -First 1).FullName $out
    Copy-Item (Get-ChildItem $dir -Recurse -Filter 7z.dll | Select-Object -First 1).FullName $out
}

if (-not (Test-Path "$out\ffmpeg.exe")) {
    $zip = Get-File "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip" "ffmpeg-essentials.zip"
    $dir = New-Clean "ffmpeg"
    tar.exe -xf $zip -C $dir
    foreach ($tool in "ffmpeg.exe", "ffprobe.exe") {
        Copy-Item (Get-ChildItem $dir -Recurse -Filter $tool | Select-Object -First 1).FullName $out
    }
}

if (-not (Test-Path "$out\magick.exe")) {
    $archive = Get-ReleaseAsset "ImageMagick/ImageMagick" "portable-Q16-HDRI-x64\.(7z|zip)$"
    $dir = New-Clean "imagemagick"
    & "$out\7z.exe" x $archive "-o$dir" -y | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "ImageMagick extract failed" }
    $home_ = (Get-ChildItem $dir -Recurse -Filter magick.exe | Select-Object -First 1).Directory.FullName
    Get-ChildItem $home_ -File | Where-Object { $_.Extension -in ".exe", ".dll", ".xml", ".icc", ".icm" -and $_.Name -notmatch "^(compare|composite|conjure|identify|mogrify|montage|stream|dcraw|ffmpeg|hp2xx|IMDisplay)" } |
        Copy-Item -Destination $out
}

if (-not (Test-Path "$out\pdfium.dll")) {
    $tgz = Get-ReleaseAsset "bblanchon/pdfium-binaries" "^pdfium-win-x64\.tgz$"
    $dir = New-Clean "pdfium"
    tar.exe -xzf $tgz -C $dir
    Copy-Item "$dir\bin\pdfium.dll" $out
}

if (-not (Test-Path "$out\assimp.exe")) {
    $release = Invoke-RestMethod "https://api.github.com/repos/assimp/assimp/releases/latest" -Headers @{ "User-Agent" = "corolla-build" }
    $zip = Get-File $release.zipball_url "assimp-$($release.tag_name).zip"
    $dir = New-Clean "assimp"
    tar.exe -xf $zip -C $dir
    $src = (Get-ChildItem $dir -Directory | Select-Object -First 1).FullName
    $cmake = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
    if (Get-Command cmake -ErrorAction SilentlyContinue) { $cmake = "cmake" }
    $build = Join-Path $dir "build"
    & $cmake -S $src -B $build -G "Visual Studio 17 2022" -A x64 `
        -DBUILD_SHARED_LIBS=OFF -DUSE_STATIC_CRT=ON -DASSIMP_BUILD_ASSIMP_TOOLS=ON -DASSIMP_BUILD_TESTS=OFF `
        -DASSIMP_BUILD_SAMPLES=OFF -DASSIMP_WARNINGS_AS_ERRORS=OFF -DASSIMP_INSTALL=OFF -DASSIMP_NO_EXPORT=OFF
    if ($LASTEXITCODE -ne 0) { throw "Assimp configure failed" }
    & $cmake --build $build --config Release --target assimp_cmd --parallel
    if ($LASTEXITCODE -ne 0) { throw "Assimp build failed" }
    Copy-Item (Get-ChildItem $build -Recurse -Filter assimp.exe | Select-Object -First 1).FullName $out
}

# magick.exe needs this Microsoft runtime to start, and a fresh PC may not have it.
if (-not (Test-Path "$out\vcomp140.dll")) {
    Copy-Item "$env:WINDIR\System32\vcomp140.dll" $out
}

# STEP files. Only the command-line converter and what it loads, in its own folder
# because it needs its own versions of common DLLs.
if (-not (Test-Path "$out\mayo\mayo-conv.exe")) {
    $version = "0.10.0"
    $name = "Mayo-$version-win64-binaries.zip"
    $zip = Get-File "https://github.com/fougue/mayo/releases/download/v$version/$name" $name
    $expected = "4cd2df02e0d5b113873515b0e222e9944629d432fe731537204a119c3b6d74b2"
    if ((Get-FileHash $zip -Algorithm SHA256).Hash -ne $expected) {
        Remove-Item $zip
        throw "Mayo download failed its checksum"
    }
    $dir = New-Clean "mayo"
    tar.exe -xf $zip -C $dir
    $from = (Get-ChildItem $dir -Recurse -Filter mayo-conv.exe | Select-Object -First 1).Directory.FullName
    New-Item -ItemType Directory -Force "$out\mayo" | Out-Null
    Get-ChildItem $from -File | Where-Object { $_.Name -notmatch "^(mayo\.exe|qt\.conf|Qt5(Gui|Svg|Widgets|WinExtras)\.dll)$" } |
        Copy-Item -Destination "$out\mayo"
    Copy-Item "$from\OpenCascade" "$out\mayo" -Recurse
}

# Fonts: FontForge redraws letters between TTF and OTF curves. Its official
# installer is too new to unpack without running it, so it's installed into a
# private folder, copied, and uninstalled again. Windows may ask for permission.
if (-not (Test-Path "$out\fontforge\bin\fontforge.exe")) {
    $installed = "C:\Program Files\FontForgeBuilds"
    if (Test-Path "$installed\bin\fontforge.exe") {
        Copy-Item $installed "$out\fontforge" -Recurse
    } else {
        $name = "FontForge-2025-10-09-Windows-x64.exe"
        $setup = Get-File "https://github.com/fontforge/fontforge/releases/download/20251009/$name" $name
        $expected = "548523f08834e344bda69abb759e30c0f84a1a5ef9a5e965eb946d86a11118a3"
        if ((Get-FileHash $setup -Algorithm SHA256).Hash -ne $expected) {
            Remove-Item $setup
            throw "FontForge download failed its checksum"
        }
        $dir = Join-Path $cache "fontforge-install"
        if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
        $arguments = "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/CURRENTUSER", "/NOICONS", "/DIR=`"$dir`""
        Start-Process $setup -ArgumentList $arguments -Wait
        if (-not (Test-Path "$dir\bin\fontforge.exe")) { throw "FontForge didn't install" }
        New-Item -ItemType Directory -Force "$out\fontforge" | Out-Null
        Get-ChildItem $dir | Where-Object { $_.Name -notlike "unins*" } |
            Copy-Item -Destination "$out\fontforge" -Recurse
        $uninstaller = Get-ChildItem $dir -Filter "unins*.exe" | Select-Object -First 1
        if ($uninstaller) {
            Start-Process $uninstaller.FullName -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait
        }
    }
    # Only the editor's window needs these; converting from the command line doesn't.
    foreach ($part in "lib\tcl8.6", "lib\tk8.6", "share\locale") {
        $path = Join-Path "$out\fontforge" $part
        if (Test-Path $path) { Remove-Item -Recurse -Force $path }
    }
}

Write-Host "Engines ready:"
Get-ChildItem $out -Filter *.exe | ForEach-Object { Write-Host "  $($_.Name)  $([math]::Round($_.Length/1MB,1)) MB" }
