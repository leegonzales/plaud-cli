# plaud installer for Windows (PowerShell).
#
#   irm https://raw.githubusercontent.com/leegonzales/plaud-cli/main/install.ps1 | iex
#
# Downloads the prebuilt Windows binary from the latest GitHub release and
# installs it. Override the location with $env:PLAUD_INSTALL_DIR. Falls back to
# `cargo install` from source if the download fails.
$ErrorActionPreference = 'Stop'

$Repo   = 'leegonzales/plaud-cli'
$Target = 'x86_64-pc-windows-msvc'
$Asset  = "plaud-$Target.zip"
$Url    = "https://github.com/$Repo/releases/latest/download/$Asset"

$InstallDir = if ($env:PLAUD_INSTALL_DIR) { $env:PLAUD_INSTALL_DIR } `
              else { Join-Path $env:LOCALAPPDATA 'Programs\plaud' }

function Install-Prebuilt {
    Write-Host "Downloading plaud ($Target)..."
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    $zip = Join-Path $tmp $Asset
    Invoke-WebRequest -Uri $Url -OutFile $zip
    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    if (-not (Test-Path (Join-Path $tmp 'plaud.exe'))) { throw 'binary not found in archive' }
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item (Join-Path $tmp 'plaud.exe') (Join-Path $InstallDir 'plaud.exe') -Force
    Remove-Item -Recurse -Force $tmp
}

function Install-FromSource {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "no prebuilt binary available and cargo is not installed.`n" +
              "Install Rust from https://rustup.rs and re-run, or download a release manually:`n" +
              "https://github.com/$Repo/releases/latest"
    }
    Write-Host 'No prebuilt binary found — building from source with cargo...'
    cargo install --git "https://github.com/$Repo" --root $InstallDir plaud
}

try {
    Install-Prebuilt
    Write-Host "Installed plaud -> $InstallDir\plaud.exe"
} catch {
    Install-FromSource
}

# PATH guidance
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$InstallDir*") {
    Write-Host ''
    Write-Host "Note: $InstallDir is not on your PATH. Add it for this user with:"
    Write-Host "  [Environment]::SetEnvironmentVariable('Path', `"$InstallDir;`$env:Path`", 'User')"
    Write-Host '  (then restart your terminal)'
}
Write-Host ''
Write-Host "Next: run 'plaud login' to sign in to Plaud."
