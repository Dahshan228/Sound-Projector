# Builds target\installer\SoundProjectorSetup.exe.
# Needs the Rust toolchain and WinLibs on PATH (see README: Building) and Inno Setup 6 (winget install JRSoftware.InnoSetup).
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
Set-Location $root

cargo build --release
if ($LASTEXITCODE) { throw 'cargo build failed' }

# VB-CABLE goes inside the installer; VB-Audio's licensing allows bundling it with credit (see setup.iss).
$cable = "$root\target\vbcable"
if (-not (Test-Path "$cable\VBCABLE_Setup_x64.exe")) {
    New-Item -ItemType Directory -Force $cable | Out-Null
    Invoke-WebRequest 'https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip' -OutFile "$cable.zip"
    Expand-Archive "$cable.zip" $cable -Force
}

$icon = Get-ChildItem "$root\target\release\build" -Recurse -Filter icon.ico | Sort-Object LastWriteTime | Select-Object -Last 1
$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 not found: winget install JRSoftware.InnoSetup' }

& $iscc /Q "/DAppVersion=$version" "/DAppIcon=$($icon.FullName)" "$PSScriptRoot\setup.iss"
if ($LASTEXITCODE) { throw 'Inno Setup failed' }
Get-Item "$root\target\installer\SoundProjectorSetup.exe"
