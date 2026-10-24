# Windows build of Modbus Poll for direct download: an NSIS installer and a portable zip.
#
#   pwsh scripts/build-windows.ps1 -Arch x64      # build\modbus-poll-windows\x64\
#   pwsh scripts/build-windows.ps1 -Arch arm64    # cross-compiled on an x64 runner (needs the MSVC ARM64 build tools)
#
# Output (in build\modbus-poll-windows\<arch>\):
#   Modbus-Poll-<version>-windows-<arch>-setup.exe      NSIS installer (per-user, no admin rights)
#   Modbus-Poll-<version>-windows-<arch>-portable.zip   Modbus Poll.exe + README.txt + LICENSE, no install
#
# The build uses the `keys` feature: Pro unlocks with a Voltrus licence key (crates/mt-store `key`). Nothing is
# signed: there is no code-signing certificate, see docs/release.md. Run it from the repository root; needs bun,
# the stable Rust toolchain and the MSVC build tools.
param(
  [ValidateSet('x64', 'arm64')][string]$Arch = 'x64'
)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $root

$triple = if ($Arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
$appDir = Join-Path $root 'apps/modbus-poll'
$conf = Get-Content (Join-Path $appDir 'src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
# The Windows build has its own version (the Mac and iOS builds read tauri.conf.json only): tauri.windows.conf.json
# overrides it when present, exactly as Tauri merges it into the config of a Windows build.
$winConf = Join-Path $appDir 'src-tauri/tauri.windows.conf.json'
$version = $conf.version
if (Test-Path $winConf) {
  $overlay = Get-Content $winConf -Raw | ConvertFrom-Json
  if ($overlay.version) { $version = $overlay.version }
}
$product = $conf.productName
$out = Join-Path $root "build/modbus-poll-windows/$Arch"
Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $out | Out-Null

rustup target add $triple
if ($LASTEXITCODE -ne 0) { throw 'rustup target add failed' }

Push-Location $appDir
try {
  bun run tauri build --features keys --target $triple --bundles nsis
  if ($LASTEXITCODE -ne 0) { throw "tauri build failed ($LASTEXITCODE)" }
} finally { Pop-Location }

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$release = Join-Path $targetDir "$triple/release"
$setup = Get-ChildItem (Join-Path $release 'bundle/nsis') -Filter '*-setup.exe' | Select-Object -First 1
if (-not $setup) { throw "no NSIS installer under $release\bundle\nsis" }
Copy-Item $setup.FullName (Join-Path $out "Modbus-Poll-$version-windows-$Arch-setup.exe")

# Portable: the same exe the installer ships, renamed to the product name, with a short README.
$stage = Join-Path $out 'stage/Modbus Poll'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Copy-Item (Join-Path $release 'modbus_poll.exe') (Join-Path $stage "$product.exe")
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $stage 'LICENSE')
@"
Modbus Poll $version for Windows ($Arch), portable

Run "$product.exe". Nothing is installed; delete the folder to remove it.

Windows SmartScreen: this build is not code-signed, so Windows may show "Windows protected your PC" the first
time. Choose "More info", then "Run anyway". Check the file against SHA256SUMS.txt from the release page first:
  certutil -hashfile "Modbus-Poll-$version-windows-$Arch-portable.zip" SHA256

Needs the Microsoft Edge WebView2 runtime (preinstalled on Windows 11 and current Windows 10;
https://developer.microsoft.com/microsoft-edge/webview2/ otherwise).

The app is free with limits; Pro unlocks with a licence key bought at https://voltrus.id/modbus-poll/windows/
(Info, then Enter key). Source code (GPL-3.0-or-later): https://github.com/VoltrusID/voltrus-explorers
"@ | Set-Content -Encoding utf8 (Join-Path $stage 'README.txt')
Compress-Archive -Path $stage -DestinationPath (Join-Path $out "Modbus-Poll-$version-windows-$Arch-portable.zip")
Remove-Item (Join-Path $out 'stage') -Recurse -Force

Get-ChildItem $out | ForEach-Object { '{0}  {1:N1} MB' -f $_.Name, ($_.Length / 1MB) }
