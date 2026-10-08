# Installe la dernière version d'EasyTab sous Windows (PowerShell) :
#   irm https://github.com/AdamSellin/EasyTab/releases/latest/download/install.ps1 | iex
# Dépôt privé : depuis un clone du dépôt,
#   powershell -ExecutionPolicy Bypass -File scripts\install.ps1
# Le script se sert alors des identifiants GitHub de git (ou de
# $env:GITHUB_TOKEN, ou de gh).
# Variables : $env:EASYTAB_VERSION (ex. v0.2.0, défaut : la dernière),
# $env:EASYTAB_LANG (fr ou en, par defaut : langue de Windows).
$ErrorActionPreference = 'Stop'
# Le fichier reste en ASCII : Windows PowerShell 5.1 lit les scripts sans BOM
# comme de l'ANSI, et `irm | iex` ne supporte pas de BOM. Les lettres
# accentuées des messages sont donc écrites par leur code.
$e = [char]0xE9; $a = [char]0xE0
# Messages en anglais, ou en francais si Windows l'est ($env:EASYTAB_LANG
# passe avant, comme pour easytab lui-meme).
if ($env:EASYTAB_LANG) {
    $fr = $env:EASYTAB_LANG -like 'fr*'
} else {
    $fr = (Get-UICulture).TwoLetterISOLanguageName -eq 'fr'
}
function Tr($en, $french) { if ($fr) { $french } else { $en } }
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
function Get-PrivateAsset($name, $dest) {
    $token = Get-GitHubToken
    if (-not $token) { return $false }
    $headers = @{ Authorization = "Bearer $token"; 'User-Agent' = 'easytab-install' }
    $info = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/$release" -Headers $headers -UseBasicParsing
    $found = $info.assets | Where-Object { $_.name -eq $name } | Select-Object -First 1
    if (-not $found) { return $false }
    & curl.exe -fsSL -H "Authorization: Bearer $token" -H 'Accept: application/octet-stream' -H 'User-Agent: easytab-install' -o $dest $found.url
    return ($LASTEXITCODE -eq 0)
}

# Downloads a release asset: direct link first, then the GitHub API (private
# repository), then gh. Returns $false if every way failed.
function Get-Asset($name, $dest) {
    $direct = $url.Substring(0, $url.LastIndexOf('/') + 1) + $name
    try {
        Invoke-WebRequest $direct -OutFile $dest -UseBasicParsing
        return $true
    } catch {}
    try {
        if (Get-PrivateAsset $name $dest) { return $true }
    } catch {}
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { return $false }
    $tag = @()
    if ($env:EASYTAB_VERSION) { $tag = @($env:EASYTAB_VERSION) }
    & gh release download @tag -R $repo -p $name -O $dest 2>$null
    return ($LASTEXITCODE -eq 0)
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("easytab-" + [Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    $zip = "$tmp\easytab.zip"
    Write-Host (Tr "Downloading $url" "T${e}l${e}chargement de $url")
    if (-not (Get-Asset $asset $zip)) {
        throw (Tr "Download failed. Private repository: connect git to GitHub, or set `$env:GITHUB_TOKEN." `
            "T${e}l${e}chargement impossible. D${e}p$([char]0xF4)t priv${e} : connecte git $a GitHub, ou d${e}finis `$env:GITHUB_TOKEN.")
    }
    # Check the archive against the release's SHA256SUMS before extracting
    # it. Releases before SHA256SUMS existed (v0.1.11 and older): warn and
    # go on.
    $sums = "$tmp\SHA256SUMS"
    if (Get-Asset 'SHA256SUMS' $sums) {
        $expected = $null
        foreach ($line in Get-Content $sums) {
            $parts = $line.Trim() -split '\s+', 2
            if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $asset) {
                $expected = $parts[0].ToLower()
                break
            }
        }
        $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
        if ($expected -ne $actual) {
            throw (Tr "$asset does not match its SHA-256 in SHA256SUMS (download corrupted or tampered with). Nothing was installed." `
                "$asset ne correspond pas $a son SHA-256 dans SHA256SUMS (t${e}l${e}chargement corrompu ou modifi${e}). Rien n'a ${e}t${e} install${e}.")
        }
        Write-Host (Tr 'SHA-256 checked' "SHA-256 v${e}rifi${e}")
    } else {
        Write-Warning (Tr 'No SHA256SUMS in this release, archive not checked' `
            "Pas de SHA256SUMS dans cette version, archive non v${e}rifi${e}e")
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
