# Builds Mentis and copies EXE + DLLs into a portable program folder (default: dist\Mentis).
#   .\scripts\Package.ps1             build and package
#   .\scripts\Package.ps1 -NoBuild    package the existing release build
#   .\scripts\Package.ps1 -CpuOnly    build without the GPU module (no Vulkan SDK needed)
# Checks the portability rules: no VC++ runtime DLLs needed, and only the optional GPU module
# ggml-vulkan.dll may import vulkan-1.dll (without a Vulkan driver Mentis falls back to the CPU).
param(
    [switch]$NoBuild,
    [switch]$CpuOnly,
    [string]$Output = 'dist\Mentis'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

if (-not (Get-Command dumpbin.exe -ErrorAction SilentlyContinue)) {
    . (Join-Path $PSScriptRoot 'Enter-DevEnvironment.ps1')
}
$triple = 'x86_64-pc-windows-msvc'
if (-not $NoBuild) {
    # cargo reports progress on stderr; with 'Stop' a redirected run would abort on it.
    $ErrorActionPreference = 'Continue'
    $features = @()
    if ($CpuOnly) { $features += '--no-default-features' }
    cargo build --release --locked --target $triple @features
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)." }
}
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$release = Join-Path $targetDir "$triple\release"
$exe = Join-Path $release 'mentis.exe'
if (-not (Test-Path $exe)) { throw "$exe is missing. Run without -NoBuild." }
# A GPU module left over from an earlier build must not slip into a CPU-only package.
$dlls = @(Get-ChildItem $release -Filter '*.dll' | Where-Object { -not ($CpuOnly -and $_.Name -eq 'ggml-vulkan.dll') })
foreach ($required in 'transcribe.dll', 'ggml.dll', 'ggml-base.dll') {
    if ($dlls.Name -notcontains $required) { throw "$required is missing in $release (feature dynamic-backends active?)." }
}
if (-not ($dlls.Name -like 'ggml-cpu-*.dll')) { throw "No CPU modules (ggml-cpu-*.dll) in $release." }

$dist = Join-Path $root $Output
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory $dist | Out-Null
Copy-Item $exe $dist
$dlls | Copy-Item -Destination $dist

$problems = @()
foreach ($file in Get-ChildItem $dist -Include '*.exe', '*.dll' -Recurse) {
    $imports = @(& dumpbin.exe /nologo /dependents $file.FullName |
        Where-Object { $_ -match '^\s+\S+\.dll\s*$' } | ForEach-Object { $_.Trim() })
    if ($imports -contains 'vulkan-1.dll' -and $file.Name -ne 'ggml-vulkan.dll') {
        $problems += "$($file.Name) imports vulkan-1.dll directly."
    }
    foreach ($crt in $imports -match '^(msvcp|vcruntime|concrt|vcomp)\d*.*\.dll$') {
        $problems += "$($file.Name) needs $crt (VC++ Redistributable)."
    }
}
if ($problems) { throw "Not portable:`n$($problems -join "`n")" }

# Texts for testers: UTF-8 with BOM and CRLF so every Notepad shows the umlauts.
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
$utf8 = New-Object System.Text.UTF8Encoding $true
function Write-Text([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, ($Text -replace "`r?`n", "`r`n"), $utf8)
}
$readme = [IO.File]::ReadAllText((Join-Path $root 'packaging\README.txt'))
Write-Text (Join-Path $dist 'README.txt') $readme.Replace('{VERSION}', $version)

# License texts of everything compiled into mentis.exe and the DLLs. Build-only crates
# (proc macros, build scripts) are not shipped and therefore not listed.
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$sources = Get-ChildItem (Join-Path $cargoHome 'registry\src') -Directory
$ErrorActionPreference = 'Continue'
$crates = cargo tree -e normal,no-proc-macro --target $triple --prefix none --locked -f '{p}|{l}' 2>$null |
    ForEach-Object { $_ -replace ' \(\*\)$', '' } | Sort-Object -Unique
$ErrorActionPreference = 'Stop'
$sections = @()
foreach ($line in $crates) {
    if ($line -notmatch '^(\S+) v(\S+)[^|]*\|\s*(.*)$') { continue }
    $name, $ver, $license = $Matches[1], $Matches[2], $Matches[3].Trim()
    if ($name -eq 'mentis') { continue }
    $dir = $sources | ForEach-Object { Join-Path $_.FullName "$name-$ver" } |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $dir) { throw "Sources of $name $ver not found (cargo fetch?)." }
    $all = @(Get-ChildItem $dir -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|NOTICE)' })
    # For "MIT OR Apache-2.0" the MIT text suffices; Apache-only crates need theirs (and NOTICE).
    $chosen = @(if ($license -match 'MIT') {
        $all | Where-Object { $_.Name -match 'MIT' }
    } else {
        $all | Where-Object { $_.Name -match 'APACHE' }
    })
    if (-not $chosen) { $chosen = @($all | Where-Object { $_.Name -notmatch 'NOTICE' }) }
    $chosen += @($all | Where-Object { $_.Name -match 'NOTICE' })
    if ($name -eq 'transcribe-cpp-sys') {
        # Vendors transcribe.cpp, ggml, miniz and Silero VAD; their texts are collected here.
        $chosen += Get-Item (Join-Path $dir 'THIRD-PARTY-LICENSES.md')
    }
    $texts = @($chosen | Select-Object -Unique | ForEach-Object { [IO.File]::ReadAllText($_.FullName).Trim() })
    if (-not $texts) {
        # A few crates ship no license file; for MIT the standard text with the authors from
        # Cargo.toml fulfils the notice requirement.
        if ($license -notmatch 'MIT') { throw "No license file in $dir ($license)." }
        $manifest = Get-Content (Join-Path $dir 'Cargo.toml') -Raw
        $authors = if ($manifest -match '(?m)^authors = \[(.*)\]') { $Matches[1] -replace '"', '' } else { "the $name authors" }
        $source = if ($manifest -match '(?m)^repository = "(.*)"') { $Matches[1] } else { 'crates.io' }
        $mit = [IO.File]::ReadAllText((Join-Path $root 'packaging\MIT.txt')).Replace('{COPYRIGHT}', $authors).Trim()
        $texts = @("(No license file in the crate; MIT text per its Cargo.toml, source: $source)", $mit)
    }
    $sections += "==============================================================================`r`n" +
        "$name $ver ($license)`r`n" +
        "==============================================================================`r`n`r`n" +
        ($texts -join "`r`n`r`n")
}
$header = [IO.File]::ReadAllText((Join-Path $root 'packaging\LICENSES-header.txt')).Replace('{VERSION}', $version).Trim()
# Mentis' own license comes first.
$own = "==============================================================================`r`n" +
    "Mentis $version (MIT)`r`n" +
    "==============================================================================`r`n`r`n" +
    [IO.File]::ReadAllText((Join-Path $root 'LICENSE')).Trim()
Write-Text (Join-Path $dist 'LICENSES.txt') ((@($header, $own) + $sections) -join "`r`n`r`n")
Write-Host ("Mentis: LICENSES.txt with Mentis and {0} components" -f $sections.Count)

# Zip with an "Mentis" folder inside, so unpacking never scatters files.
# Entries are written one by one: CreateFromDirectory in Windows PowerShell stores "\" as path
# separator, which some unzip tools turn into odd file names.
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$zip = Join-Path (Split-Path $dist -Parent) "Mentis-$version.zip"
if (Test-Path $zip) { Remove-Item $zip }
$folder = Split-Path $dist -Leaf
$archive = [IO.Compression.ZipFile]::Open($zip, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($file in Get-ChildItem $dist -File) {
        [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $file.FullName,
            "$folder/$($file.Name)", [IO.Compression.CompressionLevel]::Optimal)
    }
} finally {
    $archive.Dispose()
}
Write-Host ("Mentis: {0} ({1:N0} MB)" -f $zip, ((Get-Item $zip).Length / 1MB))

$files = Get-ChildItem $dist
$size = ($files | Measure-Object Length -Sum).Sum / 1MB
$files | Format-Table Name, @{ n = 'MB'; e = { '{0:N1}' -f ($_.Length / 1MB) } } -AutoSize | Out-Host
Write-Host ("Mentis: {0} files, {1:N0} MB in {2}" -f $files.Count, $size, $dist)
if ($files.Name -notcontains 'ggml-vulkan.dll') {
    Write-Host 'Mentis: without ggml-vulkan.dll (built without feature vulkan), CPU only.'
}
