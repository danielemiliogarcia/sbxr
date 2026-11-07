param(
    [string]$Version = $env:SBXR_VERSION,
    [string]$InstallDir = $env:SBXR_INSTALL_DIR
)

$ErrorActionPreference = "Stop"
$Repository = "danielemiliogarcia/sbxr"
$Workflow = "$Repository/.github/workflows/release.yml"
$ReleaseDir = $env:SBXR_RELEASE_DIR

if ([string]::IsNullOrWhiteSpace($Version)) {
    if (-not [string]::IsNullOrWhiteSpace($ReleaseDir)) {
        throw "Set SBXR_VERSION when using SBXR_RELEASE_DIR."
    }
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        throw "GitHub CLI (gh) is required to discover, download, and verify the latest release."
    }
    $Version = (& gh api "repos/$Repository/releases?per_page=100" --jq '[.[] | select(.draft == false and .prerelease == false and any(.assets[]; .name == "install.ps1"))][0].tag_name' | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "Could not discover the latest sbxr release."
    }
    if ([string]::IsNullOrWhiteSpace($Version)) {
        throw "No sbxr release containing install.ps1 was found."
    }
}
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$') {
    throw "Invalid sbxr version: $Version"
}
if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [System.Runtime.InteropServices.Architecture]::X64) {
    throw "This installer currently supports Windows x86-64."
}
if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $InstallDir = Join-Path $env:LOCALAPPDATA "sbxr\bin"
}
if (-not [System.IO.Path]::IsPathRooted($InstallDir)) {
    throw "SBXR_INSTALL_DIR must be an absolute path."
}

$Target = "x86_64-pc-windows-msvc"
$Name = "sbxr-$Version-$Target"
$Archive = "$Name.zip"
$Temporary = Join-Path ([System.IO.Path]::GetTempPath()) ("sbxr-install-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $Temporary | Out-Null

try {
    $ArchivePath = Join-Path $Temporary $Archive
    $ChecksumsPath = Join-Path $Temporary "SHA256SUMS"
    if (-not [string]::IsNullOrWhiteSpace($ReleaseDir)) {
        if (-not [System.IO.Path]::IsPathRooted($ReleaseDir)) {
            throw "SBXR_RELEASE_DIR must be an absolute path."
        }
        Write-Warning "Local release mode; GitHub provenance verification is skipped."
        Copy-Item (Join-Path $ReleaseDir $Archive) $ArchivePath
        Copy-Item (Join-Path $ReleaseDir "SHA256SUMS") $ChecksumsPath
    }
    else {
        if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
            throw "GitHub CLI (gh) is required to download and verify release provenance."
        }
        & gh attestation verify --help *> $null
        if ($LASTEXITCODE -ne 0) {
            throw "Update GitHub CLI (gh): this installer requires 'gh attestation verify' support."
        }
        & gh release download $Version --repo $Repository --pattern $Archive --pattern SHA256SUMS --dir $Temporary
        if ($LASTEXITCODE -ne 0) { throw "gh release download failed." }
        & gh attestation verify $ArchivePath --repo $Repository --signer-workflow $Workflow --source-ref "refs/tags/$Version" --deny-self-hosted-runners
        if ($LASTEXITCODE -ne 0) { throw "GitHub provenance verification failed." }
    }

    $ChecksumMatches = @(Get-Content $ChecksumsPath | ForEach-Object {
        $Parts = $_ -split '\s+', 2
        if ($Parts.Count -eq 2 -and $Parts[1] -eq $Archive) { $Parts[0] }
    })
    if ($ChecksumMatches.Count -ne 1 -or $ChecksumMatches[0] -notmatch '^[0-9A-Fa-f]{64}$') {
        throw "SHA256SUMS has no unique valid entry for $Archive."
    }
    $Actual = (Get-FileHash -Algorithm SHA256 $ArchivePath).Hash.ToLowerInvariant()
    if ($Actual -ne $ChecksumMatches[0].ToLowerInvariant()) {
        throw "SHA-256 verification failed for $Archive."
    }

    Expand-Archive -LiteralPath $ArchivePath -DestinationPath $Temporary
    $Candidate = Join-Path (Join-Path $Temporary $Name) "sbxr.exe"
    if (-not (Test-Path -LiteralPath $Candidate -PathType Leaf)) {
        throw "Release archive does not contain $Name/sbxr.exe."
    }
    $Reported = (& $Candidate --version | Out-String).Trim()
    if ($Reported -ne "sbxr $Version") {
        throw "Release binary reports '$Reported', expected 'sbxr $Version'."
    }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $Destination = Join-Path $InstallDir "sbxr.exe"
    if (Test-Path -LiteralPath $Destination) {
        $Existing = Get-Item -Force -LiteralPath $Destination
        if (($Existing.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing to replace reparse-point destination: $Destination"
        }
    }
    $TemporaryDestination = Join-Path $InstallDir (".sbxr.install." + [guid]::NewGuid().ToString("N") + ".exe")
    Copy-Item -LiteralPath $Candidate -Destination $TemporaryDestination
    Move-Item -Force -LiteralPath $TemporaryDestination -Destination $Destination

    Write-Host "Installed $Reported to $Destination"
    $PathEntries = $env:PATH -split ';'
    if ($PathEntries -notcontains $InstallDir) {
        Write-Host "Note: add $InstallDir to PATH."
    }
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Temporary
}
