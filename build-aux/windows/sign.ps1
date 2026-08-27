<#
.SYNOPSIS
  Sign one or more files with Azure Trusted Signing.

.DESCRIPTION
  Runs signtool with the Azure.CodeSigning Dlib and a metadata JSON naming the
  signing account and certificate profile. Used on `commune.exe` after the
  bundle is assembled, and on the `.msi` after it is built.

  Signing matters more on Windows than the equivalent did on macOS. There, the
  ad-hoc signature the bundle carries is enough to run and a real identity was
  out of reach, so the port shipped a tarball to dodge Gatekeeper. Here there is
  a certificate, so the artifacts can be what a stranger is willing to run:
  without one, SmartScreen shows an unknown-publisher warning that most people
  are right to obey.

  If there is no metadata file, signing is SKIPPED and the build still
  succeeds, producing unsigned artifacts. That is deliberate: anyone should be
  able to build Commune for Windows, and only whoever holds the certificate can
  sign it.

.PARAMETER Files
  One or more paths to sign.

.PARAMETER MetadataPath
  The Azure signing metadata JSON: { Endpoint, CodeSigningAccountName,
  CertificateProfileName }. Defaults to $env:ARTIFACT_SIGNING_METADATA, else
  artifact-signing-metadata.json in the repository root.

.NOTES
  signtool.exe is found in the newest Windows Kit; override with
  $env:SIGNTOOL_PATH. Azure.CodeSigning.Dlib.dll is found in the Trusted Signing
  client tools; override with $env:ARTIFACT_SIGNING_DLIB.
#>
param(
    [Parameter(Mandatory = $true)] [string[]]$Files,
    [string]$MetadataPath
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)

if (-not $MetadataPath) {
    $MetadataPath = if ($env:ARTIFACT_SIGNING_METADATA) { $env:ARTIFACT_SIGNING_METADATA }
                    else { Join-Path $root 'artifact-signing-metadata.json' }
}

if (-not (Test-Path $MetadataPath)) {
    Write-Host "sign: no signing metadata at '$MetadataPath' - skipping, artifacts will be UNSIGNED." -ForegroundColor Yellow
    return
}

# --- signtool.exe, from the newest Windows Kit ------------------------------
$SignTool = $env:SIGNTOOL_PATH
if (-not $SignTool -or -not (Test-Path $SignTool)) {
    $KitsBin = "C:\Program Files (x86)\Windows Kits\10\bin"
    if (Test-Path $KitsBin) {
        $latest = Get-ChildItem $KitsBin -Directory |
            Where-Object { $_.Name -match '^\d+\.\d+\.\d+' } |
            Sort-Object { [version]($_.Name -replace '^(\d+\.\d+\.\d+).*', '$1') } -Descending |
            Select-Object -First 1
        if ($latest) {
            $cand = Join-Path $latest.FullName 'x64\signtool.exe'
            if (Test-Path $cand) { $SignTool = $cand }
        }
    }
    if (-not $SignTool) {
        $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
        if ($cmd) { $SignTool = $cmd.Source }
    }
}
if (-not $SignTool -or -not (Test-Path $SignTool)) {
    throw "sign: signtool.exe not found. Install the Windows SDK or set SIGNTOOL_PATH."
}

# --- the Azure Code Signing Dlib --------------------------------------------
$Dlib = $env:ARTIFACT_SIGNING_DLIB
if (-not $Dlib -or -not (Test-Path $Dlib)) {
    $roots = @(
        "$env:LOCALAPPDATA\Microsoft\MicrosoftArtifactSigningClientTools",
        "$env:LOCALAPPDATA\Microsoft\TrustedSigningClientTools",
        "C:\ProgramData\Microsoft\MicrosoftTrustedSigningClientTools",
        "C:\Program Files\Microsoft\Azure Artifact Signing Client Tools",
        "C:\Program Files (x86)\Microsoft\Azure Artifact Signing Client Tools",
        "C:\Program Files (x86)\Windows Kits\AzureCodeSigning"
    )
    foreach ($r in $roots) {
        if (Test-Path $r) {
            $found = Get-ChildItem -Path $r -Recurse -Filter 'Azure.CodeSigning.Dlib.dll' -ErrorAction SilentlyContinue |
                Select-Object -First 1
            if ($found) { $Dlib = $found.FullName; break }
        }
    }
}
if (-not $Dlib -or -not (Test-Path $Dlib)) {
    throw "sign: Azure.CodeSigning.Dlib.dll not found. Install the Trusted Signing client tools or set ARTIFACT_SIGNING_DLIB."
}

Write-Host "sign: signtool=$SignTool" -ForegroundColor DarkGray
Write-Host "sign: dlib=$Dlib" -ForegroundColor DarkGray

foreach ($f in $Files) {
    if (-not (Test-Path $f)) { throw "sign: file not found: $f" }
    Write-Host "Signing $f" -ForegroundColor Cyan
    # The timestamp is what keeps a signature valid after the certificate
    # expires. Without it every artifact stops verifying on the certificate's
    # expiry date rather than continuing to vouch for what was signed while it
    # was valid.
    & $SignTool sign /v /fd SHA256 `
        /tr http://timestamp.acs.microsoft.com /td SHA256 `
        /dlib $Dlib /dmdf $MetadataPath `
        $f
    if ($LASTEXITCODE -ne 0) { throw "sign: signtool failed (exit $LASTEXITCODE) for $f" }
}
