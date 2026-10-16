$ErrorActionPreference = 'Stop'
$mentisFixtureDir = Join-Path (Split-Path $PSScriptRoot -Parent) '.local\fixtures'
$mentisFixture = Join-Path $mentisFixtureDir 'jfk.wav'
$mentisExpectedHash = '59DFB9A4ACB36FE2A2AFFC14BACBEE2920FF435CB13CC314A08C13F66BA7860E'
$mentisFixtureUrl = 'https://raw.githubusercontent.com/handy-computer/transcribe.cpp/f1dcd15858c4bbba87ca97862b4e448b2cf15b8f/samples/jfk.wav'
New-Item -ItemType Directory -Path $mentisFixtureDir -Force | Out-Null
if (-not (Test-Path -LiteralPath $mentisFixture)) {
    Invoke-WebRequest $mentisFixtureUrl -OutFile $mentisFixture
}
if ((Get-FileHash -LiteralPath $mentisFixture -Algorithm SHA256).Hash -ne $mentisExpectedHash) {
    throw 'Test recording has an unexpected SHA-256. Check the file; do not use it for comparisons.'
}
Write-Output $mentisFixture
