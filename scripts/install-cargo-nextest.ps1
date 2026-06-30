$ErrorActionPreference = "Stop"

$version = if ($env:CALYX_NEXTEST_VERSION) { $env:CALYX_NEXTEST_VERSION } else { "0.9" }
if ([string]::IsNullOrWhiteSpace($version) -or $version.Contains("/") -or $version.Contains("\") -or $version.Contains("..")) {
    throw "invalid CALYX_NEXTEST_VERSION: '$version'"
}

function Verify-Nextest {
    $outputLines = @(& cargo nextest --version 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-nextest is on PATH but 'cargo nextest --version' failed: $($outputLines -join ' ')"
    }
    $firstLine = if ($outputLines.Count -gt 0) { [string]$outputLines[0] } else { "" }
    if ($firstLine -notmatch '^cargo-nextest ') {
        $output = $outputLines -join ' '
        throw "unexpected cargo-nextest version output: $output"
    }
    $cmd = Get-Command cargo-nextest -ErrorAction Stop
    Write-Host "[nextest] $($cmd.Source)"
    Write-Host "[nextest] $($outputLines -join [Environment]::NewLine)"
}

Get-Command cargo -ErrorAction Stop | Out-Null

$existing = Get-Command cargo-nextest -ErrorAction SilentlyContinue
if ($existing) {
    Verify-Nextest
    exit 0
}

$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME ".cargo" }
$cargoBinDir = Join-Path $cargoHome "bin"
New-Item -ItemType Directory -Force -Path $cargoBinDir | Out-Null

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
switch ($arch) {
    "X64" { $asset = "windows" }
    "Arm64" { $asset = "windows-arm" }
    "X86" { $asset = "windows-x86" }
    default { throw "unsupported cargo-nextest install architecture: $arch" }
}

$tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("calyx-nextest-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
try {
    $archive = Join-Path $tmpDir "cargo-nextest.zip"
    $url = "https://get.nexte.st/$version/$asset"
    Write-Host "[nextest] downloading $url"
    Invoke-WebRequest -Uri $url -OutFile $archive
    Expand-Archive -Path $archive -DestinationPath $cargoBinDir -Force
}
finally {
    Remove-Item -LiteralPath $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
}

if (-not (Get-Command cargo-nextest -ErrorAction SilentlyContinue)) {
    throw "installed cargo-nextest into '$cargoBinDir', but it is not on PATH; add it to PATH and rerun 'cargo nextest --version'"
}

Verify-Nextest
