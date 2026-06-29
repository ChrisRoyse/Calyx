param(
    [Alias("p")]
    [string]$Package,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CargoArgs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Normalize-Ccbin {
    param([Parameter(Mandatory = $true)][string]$PathValue)

    $resolved = Resolve-Path -LiteralPath $PathValue -ErrorAction SilentlyContinue
    if ($null -eq $resolved) {
        return $null
    }

    $item = Get-Item -LiteralPath $resolved.Path
    if ($item.PSIsContainer) {
        $cl = Join-Path $item.FullName "cl.exe"
        if (Test-Path -LiteralPath $cl -PathType Leaf) {
            return $item.FullName
        }
        return $null
    }

    if ($item.Name -ieq "cl.exe") {
        return $item.DirectoryName
    }
    return $null
}

function Find-MsvcCcbin {
    foreach ($name in @("NVCC_CCBIN", "FORGE_CUDA_CCBIN")) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            $ccbin = Normalize-Ccbin -PathValue $value
            if ($null -eq $ccbin) {
                Write-Error "CALYX_CUDA_CCBIN_INVALID: $name must point to cl.exe or a directory containing cl.exe; value=$value"
                exit 1
            }
            return $ccbin
        }
    }

    $pathCl = Get-Command cl.exe -ErrorAction SilentlyContinue
    if ($null -ne $pathCl) {
        return (Split-Path -Parent $pathCl.Source)
    }

    $roots = @()
    if (-not [string]::IsNullOrWhiteSpace($env:ProgramFiles)) {
        $roots += Join-Path $env:ProgramFiles "Microsoft Visual Studio"
    }
    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    if (-not [string]::IsNullOrWhiteSpace($programFilesX86)) {
        $roots += Join-Path $programFilesX86 "Microsoft Visual Studio"
    }

    $candidates = foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root -PathType Container)) {
            continue
        }
        Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue |
            ForEach-Object {
                Get-ChildItem -LiteralPath $_.FullName -Directory -ErrorAction SilentlyContinue
            } |
            ForEach-Object {
                $msvcRoot = Join-Path $_.FullName "VC\Tools\MSVC"
                if (Test-Path -LiteralPath $msvcRoot -PathType Container) {
                    Get-ChildItem -LiteralPath $msvcRoot -Directory -ErrorAction SilentlyContinue
                }
            } |
            ForEach-Object {
                $ccbin = Join-Path $_.FullName "bin\Hostx64\x64"
                $cl = Join-Path $ccbin "cl.exe"
                if (Test-Path -LiteralPath $cl -PathType Leaf) {
                    [PSCustomObject]@{
                        Version = [version]$_.Name
                        Path = $ccbin
                    }
                }
            }
    }

    $selected = $candidates | Sort-Object -Property Version, Path -Descending | Select-Object -First 1
    if ($null -eq $selected) {
        Write-Error "CALYX_CUDA_HOST_COMPILER_MISSING: nvcc requires cl.exe on Windows; install Visual Studio Build Tools with MSVC x64 tools or set NVCC_CCBIN to a Hostx64\x64 directory"
        exit 1
    }
    return $selected.Path
}

function Find-CmakeBin {
    $pathCmake = Get-Command cmake.exe -ErrorAction SilentlyContinue
    if ($null -ne $pathCmake) {
        return (Split-Path -Parent $pathCmake.Source)
    }

    $roots = @()
    if (-not [string]::IsNullOrWhiteSpace($env:ProgramFiles)) {
        $roots += Join-Path $env:ProgramFiles "Microsoft Visual Studio"
    }
    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    if (-not [string]::IsNullOrWhiteSpace($programFilesX86)) {
        $roots += Join-Path $programFilesX86 "Microsoft Visual Studio"
    }

    $candidates = foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root -PathType Container)) {
            continue
        }
        Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue |
            ForEach-Object {
                Get-ChildItem -LiteralPath $_.FullName -Directory -ErrorAction SilentlyContinue
            } |
            ForEach-Object {
                $cmakeBin = Join-Path $_.FullName "Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin"
                $cmake = Join-Path $cmakeBin "cmake.exe"
                if (Test-Path -LiteralPath $cmake -PathType Leaf) {
                    [PSCustomObject]@{
                        Path = $cmakeBin
                        Modified = (Get-Item -LiteralPath $cmake).LastWriteTimeUtc
                    }
                }
            }
    }

    $selected = $candidates | Sort-Object -Property Modified, Path -Descending | Select-Object -First 1
    if ($null -eq $selected) {
        Write-Error "CALYX_CUDA_CMAKE_MISSING: cuvs-sys requires cmake.exe for CUDA builds; install CMake or Visual Studio CMake tools and retry"
        exit 1
    }
    return $selected.Path
}

function Ensure-NvccPreprocessorFlag {
    $required = "-Xcompiler /Zc:preprocessor"
    $current = [Environment]::GetEnvironmentVariable("NVCC_PREPEND_FLAGS")
    if ([string]::IsNullOrWhiteSpace($current)) {
        $env:NVCC_PREPEND_FLAGS = $required
        return
    }
    if ($current -notmatch "/Zc:preprocessor") {
        $env:NVCC_PREPEND_FLAGS = "$required $current"
        return
    }
    $env:NVCC_PREPEND_FLAGS = $current
}

if ($CargoArgs.Count -eq 0) {
    Write-Error "CALYX_CUDA_CARGO_ARGS_MISSING: usage: scripts\cargo-cuda.ps1 <cargo args>; example: scripts\cargo-cuda.ps1 check -p calyx-cli --features cuda"
    exit 1
}

$ccbin = Find-MsvcCcbin
$cmakeBin = Find-CmakeBin
$env:NVCC_CCBIN = $ccbin
$env:FORGE_CUDA_CCBIN = $ccbin
$env:PATH = "$cmakeBin;$env:PATH"
Ensure-NvccPreprocessorFlag

Write-Host "CALYX_CUDA_ENV NVCC_CCBIN=$env:NVCC_CCBIN FORGE_CUDA_CCBIN=$env:FORGE_CUDA_CCBIN CMAKE_BIN=$cmakeBin NVCC_PREPEND_FLAGS=$env:NVCC_PREPEND_FLAGS"
$forwardedArgs = @($CargoArgs)
if (-not [string]::IsNullOrWhiteSpace($Package)) {
    if ($forwardedArgs -contains "-p" -or $forwardedArgs -contains "--package") {
        Write-Error "CALYX_CUDA_PACKAGE_DUPLICATE: package was provided both as a script parameter and a cargo argument"
        exit 1
    }
    $forwardedArgs += @("-p", $Package)
}

cargo @forwardedArgs
exit $LASTEXITCODE
