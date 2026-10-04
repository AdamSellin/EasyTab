# Installe la dernière version d'EasyTab sous Windows (PowerShell) :
#   irm https://github.com/AdamSellin/EasyTab/releases/latest/download/install.ps1 | iex
# Variables : $env:EASYTAB_VERSION (ex. v0.2.0, défaut : la dernière).
$ErrorActionPreference = 'Stop'

$repo = 'AdamSellin/EasyTab'
$target = 'x86_64-pc-windows-msvc'
if ($env:EASYTAB_VERSION) {
    $url = "https://github.com/$repo/releases/download/$($env:EASYTAB_VERSION)/easytab-$target.zip"
} else {
    $url = "https://github.com/$repo/releases/latest/download/easytab-$target.zip"
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("easytab-" + [Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Write-Host "Téléchargement de $url"
    Invoke-WebRequest $url -OutFile "$tmp\easytab.zip" -UseBasicParsing
    Expand-Archive "$tmp\easytab.zip" -DestinationPath $tmp
    $easytab = "$tmp\easytab-$target\easytab.exe"
    # Copie les programmes dans ~\.easytab\bin, puis configure PowerShell et
    # Git Bash.
    & $easytab install --shell pwsh
    & $easytab install --shell bash
} finally {
    Remove-Item -Recurse -Force $tmp
}
