# Checks the Windows build of Modbus Poll the way a customer meets it. Run on a Windows machine.
#
#   pwsh scripts/smoke-windows.ps1 -Dir build\modbus-poll-windows\x64 -Arch x64 [-LicenseServer]
#
# 1. portable zip: unpack, run `--smoke` (headless self-test: TCP master and slave on loopback, serial listing)
# 2. installer: silent install, run the installed exe with `--smoke`, start the real app and wait for its window
#    (the WebView2 frontend loading proves the bundle), take a screenshot, stop it, silent uninstall
# 3. -LicenseServer: `--smoke-license` asks the live licence server about a made-up key (no real licence is used)
#
# Exits non-zero on the first failed check. Screenshots and logs go to <Dir>\smoke\.
param(
  [Parameter(Mandatory)][string]$Dir,
  [ValidateSet('x64', 'arm64')][string]$Arch = 'x64',
  [switch]$LicenseServer
)
$ErrorActionPreference = 'Stop'
$Dir = (Resolve-Path $Dir).Path
$evidence = Join-Path $Dir 'smoke'
New-Item -ItemType Directory -Path $evidence -Force | Out-Null

function Fail($msg) { Write-Host "SMOKE FAILED: $msg"; exit 1 }

function Invoke-Smoke($exe, $flag, $name) {
  $report = Join-Path $evidence "$name.txt"
  Remove-Item $report -ErrorAction SilentlyContinue
  $p = Start-Process -FilePath $exe -ArgumentList $flag, "`"$report`"" -Wait -PassThru
  $text = if (Test-Path $report) { (Get-Content $report -Raw).Trim() } else { '' }
  Write-Host "$name -> exit $($p.ExitCode): $text"
  if ($p.ExitCode -ne 0 -or $text -notmatch 'smoke ok') { Fail "$name ($exe $flag)" }
}

function Save-Screenshot($path) {
  try {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
  } catch { Write-Host "no screenshot: $_" }
}

$zip = Get-ChildItem $Dir -Filter "*-windows-$Arch-portable.zip" | Select-Object -First 1
$setup = Get-ChildItem $Dir -Filter "*-windows-$Arch-setup.exe" | Select-Object -First 1
if (-not $zip) { Fail "no portable zip in $Dir" }
if (-not $setup) { Fail "no installer in $Dir" }

# 1. Portable
$unpacked = Join-Path $env:RUNNER_TEMP "modbus-poll-portable-$Arch"
if (-not $env:RUNNER_TEMP) { $unpacked = Join-Path $env:TEMP "modbus-poll-portable-$Arch" }
Remove-Item $unpacked -Recurse -Force -ErrorAction SilentlyContinue
Expand-Archive $zip.FullName -DestinationPath $unpacked
$portableExe = Join-Path $unpacked 'Modbus Poll/Modbus Poll.exe'
if (-not (Test-Path $portableExe)) { Fail "portable zip has no 'Modbus Poll/Modbus Poll.exe'" }
foreach ($f in 'README.txt', 'LICENSE') { if (-not (Test-Path (Join-Path $unpacked "Modbus Poll/$f"))) { Fail "portable zip has no $f" } }
Invoke-Smoke $portableExe '--smoke' 'portable-smoke'
Write-Host ("portable exe version: " + (Get-Item $portableExe).VersionInfo.ProductVersion)

# 2. Installer
$install = Start-Process -FilePath $setup.FullName -ArgumentList '/S' -Wait -PassThru
if ($install.ExitCode -ne 0) { Fail "installer exit code $($install.ExitCode)" }
# The per-user NSIS installer goes to %LOCALAPPDATA%\Programs\<product>; fall back to the uninstall registry entry.
$installDir = $null
foreach ($candidate in @((Join-Path $env:LOCALAPPDATA 'Programs\Modbus Poll'), (Join-Path $env:LOCALAPPDATA 'Modbus Poll'))) {
  if (Test-Path $candidate) { $installDir = $candidate; break }
}
if (-not $installDir) {
  $entry = Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue |
    ForEach-Object { Get-ItemProperty $_.PSPath } | Where-Object { $_.DisplayName -like 'Modbus Poll*' } | Select-Object -First 1
  if ($entry -and $entry.InstallLocation) { $installDir = $entry.InstallLocation.Trim('"') }
}
if (-not $installDir) {
  Get-ChildItem $env:LOCALAPPDATA -Directory | ForEach-Object { Write-Host "LOCALAPPDATA: $($_.Name)" }
  Fail 'could not find where the installer put Modbus Poll'
}
Write-Host "install dir: $installDir"
Get-ChildItem $installDir | ForEach-Object { Write-Host "installed: $($_.Name)" }
$installed = Join-Path $installDir 'Modbus Poll.exe'
if (-not (Test-Path $installed)) { Fail "installer did not create $installed" }
Invoke-Smoke $installed '--smoke' 'installed-smoke'
if ($LicenseServer) { Invoke-Smoke $installed '--smoke-license' 'installed-license-smoke' }

$app = Start-Process -FilePath $installed -PassThru
$window = $null
for ($i = 0; $i -lt 40 -and -not $window; $i++) {
  Start-Sleep -Seconds 1
  $app.Refresh()
  if ($app.HasExited) { Fail "the app exited with code $($app.ExitCode) before showing a window" }
  if ($app.MainWindowHandle -ne 0 -and $app.MainWindowTitle) { $window = $app.MainWindowTitle }
}
if (-not $window) { Fail 'the app showed no window within 40 s' }
Write-Host "window title: $window"
Start-Sleep -Seconds 8   # let WebView2 load the frontend
$app.Refresh()
if ($app.HasExited) { Fail "the app exited with code $($app.ExitCode) while loading" }
$webview = @(Get-Process msedgewebview2 -ErrorAction SilentlyContinue).Count
Write-Host "msedgewebview2 processes: $webview"
if ($webview -lt 1) { Fail 'WebView2 never started' }
Save-Screenshot (Join-Path $evidence "app-$Arch.png")
Stop-Process -Id $app.Id -Force

$uninstaller = Get-ChildItem $installDir -Filter 'uninstall*.exe' | Select-Object -First 1
if (-not $uninstaller) { Fail 'no uninstaller was installed' }
$un = Start-Process -FilePath $uninstaller.FullName -ArgumentList '/S' -Wait -PassThru
Start-Sleep -Seconds 3
if (Test-Path $installed) { Fail 'uninstaller left Modbus Poll.exe behind' }
Write-Host 'SMOKE OK'
