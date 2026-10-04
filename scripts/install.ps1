# Installe la dernière version d'EasyTab sous Windows (PowerShell) :
#   irm https://github.com/AdamSellin/EasyTab/releases/latest/download/install.ps1 | iex
# Dépôt privé : depuis un clone du dépôt,
#   powershell -ExecutionPolicy Bypass -File scripts\install.ps1
# Le script se sert alors des identifiants GitHub de git (ou de
# $env:GITHUB_TOKEN, ou de gh).
# Variables : $env:EASYTAB_VERSION (ex. v0.2.0, défaut : la dernière).
$ErrorActionPreference = 'Stop'
# Le fichier reste en ASCII : Windows PowerShell 5.1 lit les scripts sans BOM
# comme de l'ANSI, et `irm | iex` ne supporte pas de BOM. Les lettres
# accentuées des messages sont donc écrites par leur code.
$e = [char]0xE9; $a = [char]0xE0
# Windows PowerShell 5.1 n'active pas TLS 1.2 par défaut.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = 'AdamSellin/EasyTab'
$target = 'x86_64-pc-windows-msvc'
$asset = "easytab-$target.zip"
if ($env:EASYTAB_VERSION) {
    $url = "https://github.com/$repo/releases/download/$($env:EASYTAB_VERSION)/$asset"
    $release = "tags/$($env:EASYTAB_VERSION)"
} else {
    $url = "https://github.com/$repo/releases/latest/download/$asset"
    $release = 'latest'
}

# Jeton GitHub : $env:GITHUB_TOKEN, ou celui que git utilise déjà.
function Get-GitHubToken {
    if ($env:GITHUB_TOKEN) { return $env:GITHUB_TOKEN }
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) { return $null }
    try {
        $lines = "protocol=https`nhost=github.com`n`n" | git credential fill 2>$null
    } catch {
        return $null
    }
    foreach ($line in $lines) {
        if ($line -like 'password=*') { return $line.Substring(9) }
    }
    return $null
}

# Dépôt privé : le lien direct répond 404. On télécharge alors par l'API
# GitHub avec le jeton. curl.exe (fourni avec Windows 10 et 11) suit la
# redirection vers le stockage de GitHub sans y renvoyer le jeton.
function Get-PrivateAsset($dest) {
    $token = Get-GitHubToken
    if (-not $token) { return $false }
    $headers = @{ Authorization = "Bearer $token"; 'User-Agent' = 'easytab-install' }
    $info = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/$release" -Headers $headers -UseBasicParsing
    $found = $info.assets | Where-Object { $_.name -eq $asset } | Select-Object -First 1
    if (-not $found) { return $false }
    & curl.exe -fsSL -H "Authorization: Bearer $token" -H 'Accept: application/octet-stream' -H 'User-Agent: easytab-install' -o $dest $found.url
    return ($LASTEXITCODE -eq 0)
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("easytab-" + [Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    $zip = "$tmp\easytab.zip"
    Write-Host "T${e}l${e}chargement de $url"
    try {
        Invoke-WebRequest $url -OutFile $zip -UseBasicParsing
    } catch {
        Write-Host "Lien direct indisponible (d${e}p$([char]0xF4)t priv${e} ?), t${e}l${e}chargement avec tes identifiants GitHub"
        if (-not (Get-PrivateAsset $zip)) {
            if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
                throw "T${e}l${e}chargement impossible. D${e}p$([char]0xF4)t priv${e} : connecte git $a GitHub, ou d${e}finis `$env:GITHUB_TOKEN."
            }
            $tag = @()
            if ($env:EASYTAB_VERSION) { $tag = @($env:EASYTAB_VERSION) }
            & gh release download @tag -R $repo -p $asset -O $zip
            if ($LASTEXITCODE -ne 0) { throw "gh release download a ${e}chou${e} ($LASTEXITCODE)." }
        }
    }
    Expand-Archive $zip -DestinationPath $tmp
    $easytab = "$tmp\easytab-$target\easytab.exe"
    # Copie les programmes dans ~\.easytab\bin, puis configure PowerShell et
    # Git Bash.
    & $easytab install --shell pwsh
    & $easytab install --shell bash
} finally {
    Remove-Item -Recurse -Force $tmp
}
