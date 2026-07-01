[CmdletBinding()]
param(
    [switch]$Write
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $PSCommandPath
$RepoRoot = Resolve-Path (Join-Path $ScriptDir "..")
Set-Location $RepoRoot

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo-fmt-workspace: ERROR cargo not found on PATH"
}

$Mode = if ($Write) { "write" } else { "check" }
$MetadataJson = & cargo metadata --no-deps --format-version 1
if ($LASTEXITCODE -ne 0) {
    throw "cargo-fmt-workspace: ERROR cargo metadata failed with exit code $LASTEXITCODE"
}

$Metadata = $MetadataJson | ConvertFrom-Json -Depth 100
$PackagesById = @{}
foreach ($Package in $Metadata.packages) {
    $PackagesById[$Package.id] = $Package.name
}

$Packages = @()
foreach ($Member in $Metadata.workspace_members) {
    if (-not $PackagesById.ContainsKey($Member)) {
        throw "cargo-fmt-workspace: ERROR cargo metadata omitted workspace member '$Member'"
    }
    $Packages += $PackagesById[$Member]
}

if ($Packages.Count -eq 0) {
    throw "cargo-fmt-workspace: ERROR cargo metadata returned zero workspace packages"
}

Write-Host "cargo-fmt-workspace: mode=$Mode package_count=$($Packages.Count)"
foreach ($Package in $Packages) {
    $Args = @("fmt", "-p", $Package)
    if (-not $Write) {
        $Args += @("--", "--check")
    }

    Write-Host "cargo-fmt-workspace: package=$Package command=cargo $($Args -join ' ')"
    & cargo @Args
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-fmt-workspace: ERROR cargo fmt failed for package '$Package' with exit code $LASTEXITCODE"
    }
}
