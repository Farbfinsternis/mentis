# Dot-source from PowerShell: . ./scripts/Enter-DevEnvironment.ps1
$ErrorActionPreference = 'Stop'

$mentisCargoBin = if ($env:CARGO_HOME) {
    Join-Path $env:CARGO_HOME 'bin'
} else {
    Join-Path $env:USERPROFILE '.cargo\bin'
}
if (-not (Test-Path (Join-Path $mentisCargoBin 'cargo.exe'))) {
    throw 'Rust is missing. See README.md.'
}
$env:PATH = "$mentisCargoBin;$env:PATH"

$mentisVsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $mentisVsWhere)) {
    throw 'Visual Studio Build Tools are missing. See README.md.'
}
$mentisVsPath = & $mentisVsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $mentisVsPath) {
    throw 'No Visual Studio installation with x64 C++ build tools found.'
}
# VsDevCmd calls vswhere.exe by name; without its folder on PATH it prints a needless error.
$env:PATH = "$(Split-Path $mentisVsWhere -Parent);$env:PATH"
& (Join-Path $mentisVsPath 'Common7\Tools\Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation -NoLogo
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    throw 'Could not activate the MSVC environment.'
}

$mentisCmakeBin = Join-Path $mentisVsPath 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
if (Test-Path (Join-Path $mentisCmakeBin 'cmake.exe')) {
    $env:PATH = "$mentisCmakeBin;$env:PATH"
}
if (-not (Get-Command cmake.exe -ErrorAction SilentlyContinue)) {
    throw 'CMake is missing. Install the C++ CMake tools in the Visual Studio Installer.'
}
$mentisVulkanSdk = $env:VULKAN_SDK
if (-not $mentisVulkanSdk) {
    $mentisVulkanSdk = [Environment]::GetEnvironmentVariable('VULKAN_SDK', 'User')
}
if (-not $mentisVulkanSdk) {
    $mentisVulkanSdk = [Environment]::GetEnvironmentVariable('VULKAN_SDK', 'Machine')
}
if ($mentisVulkanSdk -and (Test-Path (Join-Path $mentisVulkanSdk 'Bin\glslc.exe'))) {
    $env:VULKAN_SDK = $mentisVulkanSdk
    $env:PATH = "$(Join-Path $mentisVulkanSdk 'Bin');$env:PATH"
    Write-Host "Mentis: Vulkan SDK $mentisVulkanSdk active."
}
$mentisLlvmBin = Join-Path $env:ProgramFiles 'LLVM\bin'
if (-not $env:LIBCLANG_PATH -and (Test-Path (Join-Path $mentisLlvmBin 'libclang.dll'))) {
    $env:LIBCLANG_PATH = $mentisLlvmBin
}
Write-Host 'Mentis: x64 MSVC, Rust and CMake environment active (this PowerShell session only).'
