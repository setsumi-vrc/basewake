# Copies Basewake to %LOCALAPPDATA%\Programs\Basewake: from the release zip it sits in, or from a
# fresh build when run in the source folder.
# Usage:  .\install.ps1            (copy)
#         .\install.ps1 -Register  (also register as a SteamVR startup app; SteamVR is contacted)
param([switch]$Register)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

if (Test-Path 'Cargo.toml') {
    # cargo writes progress to stderr; run it through cmd so Windows PowerShell does not treat that as an error.
    cmd /c "cargo build --release 2>&1"
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
    $src = 'target\release'
    $license = 'third_party\openvr-2.12.14\LICENSE'
} else {
    $src = '.'
    $license = 'openvr_api.LICENSE.txt'
}

$dest = Join-Path $env:LOCALAPPDATA 'Programs\Basewake'
New-Item -ItemType Directory -Force $dest | Out-Null

# The app used to be "Lighthouse Manager": stop that copy too, so only one talks to Bluetooth.
$old = Join-Path $env:LOCALAPPDATA 'Programs\LighthouseManager'
if (Get-Process lighthouse -ErrorAction SilentlyContinue) {
    if (Test-Path "$old\lighthousectl.exe") { cmd /c "`"$old\lighthousectl.exe`" quit 2>&1" }
    Start-Sleep -Seconds 1
    Get-Process lighthouse -ErrorAction SilentlyContinue | Stop-Process -Force
}

# Close a running copy so its files can be replaced.
if (Get-Process basewake -ErrorAction SilentlyContinue) {
    cmd /c "`"$dest\basewakectl.exe`" quit 2>&1"
    Start-Sleep -Seconds 1
}

foreach ($f in 'basewake.exe', 'basewakectl.exe', 'openvr_api.dll') {
    Copy-Item (Join-Path $src $f) $dest -Force
}
Copy-Item $license "$dest\openvr_api.LICENSE.txt" -Force
Write-Host "Installed to $dest"

if ($Register) {
    & "$dest\basewakectl.exe" install
    # Registering also drops the old Lighthouse Manager entry from SteamVR; its files can go now.
    if ($LASTEXITCODE -eq 0 -and (Test-Path $old)) {
        Remove-Item -Recurse -Force $old
        Write-Host "Removed the old Lighthouse Manager from $old"
    }
} elseif (Test-Path $old) {
    Write-Host "The old Lighthouse Manager is still in $old. Run .\install.ps1 -Register to replace it in SteamVR."
}

