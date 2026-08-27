<#
.SYNOPSIS
  Build (and optionally sign) the Commune MSI from an assembled bundle.

.DESCRIPTION
  Signs the executable, builds the MSI around the bundle folder with WiX 5, and
  signs the MSI. Both signing steps are skipped when there is no signing
  metadata, so this works for anyone who does not hold the certificate.

  The order matters: the executable is signed BEFORE the MSI is built, because
  the MSI embeds a copy of it and a signature applied afterwards would not reach
  the copy inside the package.

  The bundle has to exist first:

      meson compile -C _build windows-bundle
      pwsh -File build-aux\windows\build-msi.ps1 -BundleDir "_build\windows\Commune Devel"

.PARAMETER BundleDir
  The folder `bundle.sh` assembled.

.PARAMETER Profile
  Devel, Beta or Stable. Decides the name, the application ID and — most
  importantly — the upgrade code, which is what lets a development build and a
  stable one be installed at the same time instead of replacing each other.

.PARAMETER Version
  The product version. Defaults to the version in Cargo.toml.

.PARAMETER OutDir
  Where to write the MSI. Defaults to the bundle's parent directory.

.NOTES
  Needs the WiX 5 dotnet tool and its UI extension:

      dotnet tool install --global wix --version 5.*
      wix extension add --global WixToolset.UI.wixext/5.0.0
#>
param(
    [Parameter(Mandatory = $true)] [string]$BundleDir,
    [ValidateSet('Devel', 'Beta', 'Stable')] [string]$Profile = 'Devel',
    [string]$Version,
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$root = Split-Path -Parent (Split-Path -Parent $here)

if (-not (Test-Path $BundleDir)) { throw "build-msi: no such bundle directory: $BundleDir" }
$BundleDir = (Resolve-Path $BundleDir).Path

$exe = Join-Path $BundleDir 'bin\commune.exe'
if (-not (Test-Path $exe)) { throw "build-msi: no commune.exe in $BundleDir\bin" }

if (-not $OutDir) { $OutDir = Split-Path -Parent $BundleDir }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# --- the version, single-sourced from Cargo.toml ----------------------------
if (-not $Version) {
    $cargo = Get-Content (Join-Path $root 'Cargo.toml') -Raw
    if ($cargo -match '(?m)^version\s*=\s*"([^"]+)"') { $Version = $Matches[1] }
    else { throw "build-msi: could not read the version from Cargo.toml" }
}
# An MSI version is three numeric fields and nothing else; anything a
# development version appends is dropped rather than passed on to fail inside
# WiX with a message about a malformed version.
$msiVersion = ($Version -split '-')[0]

# --- what the profile decides -----------------------------------------------
# These upgrade codes are permanent. A new one makes every installed copy
# invisible to the installer, so it stops upgrading and starts installing
# alongside. They are recorded in doc/rebrand.md.
$profiles = @{
    'Stable' = @{ Name = 'Commune';       AppId = 'io.github.steeb_k.Commune';       Upgrade = '9434FE95-FA50-4428-A565-E1491147CE85' }
    'Devel'  = @{ Name = 'Commune Devel'; AppId = 'io.github.steeb_k.Commune.Devel'; Upgrade = '2CF39ACA-5B32-47B8-B73F-91BC3C2512BB' }
    'Beta'   = @{ Name = 'Commune Beta';  AppId = 'io.github.steeb_k.Commune.Beta';  Upgrade = '7974C0E9-61F9-4533-90FD-D6C70EC3AA4C' }
}
$p = $profiles[$Profile]

$icon = Join-Path $root '_build\windows\commune.ico'
if (-not (Test-Path $icon)) { throw "build-msi: no icon at $icon — run 'meson setup' first" }

# WiX wants RTF for the licence page, and the repository has plain text.
$licenseRtf = Join-Path $OutDir 'license.rtf'
if (-not (Test-Path $licenseRtf)) {
    Write-Host 'build-msi: rendering LICENSE as RTF' -ForegroundColor DarkGray
    $text = (Get-Content (Join-Path $root 'LICENSE') -Raw) -replace '\\', '\\\\' -replace '([{}])', '\$1'
    $text = $text -replace "`r`n", '\par ' -replace "`n", '\par '
    "{\rtf1\ansi\deff0{\fonttbl{\f0\fnil\fcharset0 Consolas;}}\fs16 $text}" |
        Set-Content -Path $licenseRtf -Encoding ASCII
}

# --- sign the executable, before it is packaged -----------------------------
& (Join-Path $here 'sign.ps1') -Files @($exe)

# --- build ------------------------------------------------------------------
$wix = Get-Command wix -ErrorAction SilentlyContinue
if (-not $wix) {
    throw "build-msi: the wix tool is not installed. Run: dotnet tool install --global wix --version 5.*"
}

$msi = Join-Path $OutDir "$($p.Name -replace ' ', '-')-$Version-x64.msi"
Write-Host "build-msi: building $msi" -ForegroundColor Cyan

# -arch x64 is not optional: without it WiX builds a 32-bit package, and a
# 32-bit package resolves the per-user program directory differently.
& wix build -arch x64 (Join-Path $here 'commune.wxs') `
    -ext WixToolset.UI.wixext `
    -d "BundleDir=$BundleDir" `
    -d "Version=$msiVersion" `
    -d "AppName=$($p.Name)" `
    -d "AppId=$($p.AppId)" `
    -d "UpgradeCode=$($p.Upgrade)" `
    -d "IconFile=$icon" `
    -d "LicenseRtf=$licenseRtf" `
    -o $msi
if ($LASTEXITCODE -ne 0) { throw "build-msi: wix build failed (exit $LASTEXITCODE)" }

# --- sign the package -------------------------------------------------------
& (Join-Path $here 'sign.ps1') -Files @($msi)

Write-Host "build-msi: $msi" -ForegroundColor Green
