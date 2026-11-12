param(
    [switch]$SkipInstaller
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot
try {
    if (-not (Get-Command "cargo" -ErrorAction SilentlyContinue)) { throw "Rust/Cargo is required to build the Vortal command-line tools." }
    cargo build --target x86_64-pc-windows-msvc -p vortal-desktop -p vortal-cli -p vortal-daemon -p vortal-input-windows --release --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }

    if (-not $SkipInstaller) {
        $nsis = Get-Command "makensis" -ErrorAction SilentlyContinue
        if (-not $nsis) {
            $nsisPath = Join-Path ${env:ProgramFiles(x86)} "NSIS\makensis.exe"
            if (Test-Path $nsisPath) { $nsis = $nsisPath }
        }
        if (-not $nsis) { throw "NSIS is required to build the Windows installer." }
        New-Item -ItemType Directory -Force -Path (Join-Path $repoRoot "dist\windows-native") | Out-Null
        & $nsis installers\windows\Vortal.nsi
        if ($LASTEXITCODE -ne 0) { throw "NSIS packaging failed with exit code $LASTEXITCODE" }
        Write-Host "Rust installer generated at dist\windows-native\Vortal-Setup-Rust-x64.exe"
    }
}
finally {
    Pop-Location
}
