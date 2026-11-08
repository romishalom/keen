# whet: the keen installer for Windows (PowerShell bootstrap)
#
#   irm https://github.com/romishalom/keen/releases/latest/download/install.ps1 | iex
#   & ([scriptblock]::Create((irm https://github.com/romishalom/keen/releases/latest/download/install.ps1))) -y --default-toolchain 0.3.0
#
# Downloads `whet`, verifies its SHA-256, and runs `whet init` with any arguments given.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$InitArgs)

$ErrorActionPreference = 'Stop'
$Dist = if ($env:WHET_DIST_SERVER) { $env:WHET_DIST_SERVER.TrimEnd('/') } else { 'https://github.com/romishalom/keen/releases' }

if ($env:WHET_TARGET) {
    $Target = $env:WHET_TARGET
} elseif (-not [Environment]::Is64BitOperatingSystem) {
    throw 'keen needs a 64-bit version of Windows'
} elseif ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    $Target = 'aarch64-pc-windows-msvc'
} else {
    $Target = 'x86_64-pc-windows-msvc'
}

if ($env:WHET_VERSION) {
    $Version = $env:WHET_VERSION
} else {
    # GitHub releases latest download endpoint for the 'latest' asset file
    $Version = ((Invoke-RestMethod "$Dist/latest/download/latest").ToString() -split "`n")[0].Trim()
}
if ($Version -notmatch '^[0-9A-Za-z.-]+$') { throw "invalid version from the release server: '$Version'" }

# GitHub releases flat asset layout: /releases/download/v<version>/<filename>
$Url = "$Dist/download/v$Version/whet-$Target.exe"
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("whet-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
    $Exe = Join-Path $Tmp 'whet.exe'
    Write-Host "whet-install: downloading whet $Version for $Target"
    Invoke-WebRequest -Uri $Url -OutFile $Exe -UseBasicParsing
    $Want = (((Invoke-RestMethod "$Url.sha256").ToString().Trim()) -split '\s+')[0].ToLower()
    $Have = (Get-FileHash -Path $Exe -Algorithm SHA256).Hash.ToLower()
    if ($Want -ne $Have) { throw "checksum mismatch for whet (expected $Want, got $Have); nothing was installed" }
    & $Exe init @InitArgs
    exit $LASTEXITCODE
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}