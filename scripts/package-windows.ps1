# Builds Opentern for Windows: an x64 and an ARM64 zip, plus one installer
# (Opentern-windows-setup.exe) that installs the right build for the PC.
# Adapted from Magpie's. The zip names are a contract with the in-app
# updater (crates/core/src/update.rs): opentern.exe at the zip's root.
#
#   pwsh scripts/package-windows.ps1 [version]
#
# Needs the MSVC toolchain with ARM64 build tools, both Rust targets, and
# Inno Setup 6.3+ (choco install innosetup).
param([string]$Version = "")
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

# A tag like v0.1.2 gives the version; anything else (a branch name on a
# manual run) falls back to Cargo.toml.
$Version = $Version.TrimStart('v')
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    $Version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
}

$targets = [ordered]@{ 'x64' = 'x86_64-pc-windows-msvc'; 'arm64' = 'aarch64-pc-windows-msvc' }
New-Item -ItemType Directory -Force dist | Out-Null

foreach ($arch in $targets.Keys) {
    $target = $targets[$arch]
    cargo build --release --locked --target $target -p opentern
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed for $target" }

    $stage = "dist\Opentern-windows-$arch"
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
    New-Item -ItemType Directory $stage | Out-Null
    Copy-Item "target\$target\release\opentern.exe" $stage
    Copy-Item LICENSE "$stage\LICENSE.txt"
    Copy-Item README.md $stage
    Copy-Item THIRD-PARTY-LICENSES.txt $stage
    Compress-Archive -Path "$stage\*" -DestinationPath "dist\Opentern-windows-$arch.zip" -Force
    Remove-Item -Recurse -Force $stage
    Write-Host "Built dist\Opentern-windows-$arch.zip"
}

$iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $iscc) { $iscc = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe' }
if (-not (Test-Path $iscc)) { throw "Inno Setup 6 not found (choco install innosetup)" }

$root = (Get-Location).Path
& $iscc /Qp "/DVersion=$Version" `
    "/DSrcX64=$root\target\x86_64-pc-windows-msvc\release\opentern.exe" `
    "/DSrcArm64=$root\target\aarch64-pc-windows-msvc\release\opentern.exe" `
    "/DOutDir=$root\dist" `
    packaging\windows\opentern.iss
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
Write-Host "Built dist\Opentern-windows-setup.exe"
