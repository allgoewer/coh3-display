# Builds the firmware and the release exe (which carries the firmware) and wraps the exe
# into target\installer\coh3-display-<version>-setup.exe.
# Needs Inno Setup 6 (winget install JRSoftware.InnoSetup).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw 'ISCC.exe not found. Install Inno Setup 6: winget install JRSoftware.InnoSetup' }

Push-Location $root
try {
    # The app embeds the display firmware, so that is built first (needs the stable toolchain
    # with the riscv32imac-unknown-none-elf target; see firmware\rust-toolchain.toml).
    Push-Location (Join-Path $root 'firmware')
    try {
        cargo build --profile dist
        if ($LASTEXITCODE -ne 0) { throw 'firmware build failed' }
    }
    finally { Pop-Location }
    $firmware = Join-Path $root 'firmware\target\riscv32imac-unknown-none-elf\dist\firmware'
    if (-not (Test-Path $firmware)) { throw "firmware image missing: $firmware" }

    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object { $_.name -eq 'coh3-display' }).version

    # The exe's version resource only takes numbers, so a pre-release part (-rc.1) is left off there.
    $numericVersion = ($version -split '[-+]')[0]
    & $iscc "/DAppVersion=$version" "/DAppNumericVersion=$numericVersion" /Qp (Join-Path $PSScriptRoot 'coh3-display.iss') | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'ISCC failed' }

    Get-Item (Join-Path $root "target\installer\coh3-display-$version-setup.exe")
}
finally {
    Pop-Location
}
