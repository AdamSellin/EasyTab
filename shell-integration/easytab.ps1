# Integration EasyTab pour PowerShell. `easytab install --shell pwsh` copie
# ce fichier dans ~/.easytab/bin/easytab-profile.ps1, a cote de easytab-term
# (nomme easytab.ps1, il passerait avant easytab.exe), et le profil le
# charge (par `.`) deux fois : en haut, pour relancer PowerShell sous EasyTab
# avant de lire le reste, et en bas, pour poser le hook de prompt apres les
# themes (oh-my-posh, starship...). Aucun chemin n'y est ecrit : le fichier
# publie est le meme pour tous, et peut donc etre signe (politique AllSigned).
# Desactiver ponctuellement : $env:EASYTAB_DISABLE = 1, puis lancer pwsh.
# (Fichier en ASCII : Windows PowerShell 5 lit mal l'UTF-8 sans BOM.)

# `easytab init pwsh` (anciens profils) remplace ces deux lignes par le chemin.
$__easytab_term = Join-Path $PSScriptRoot 'easytab-term'
if (Test-Path -LiteralPath "$__easytab_term.exe") { $__easytab_term += '.exe' }

# Rend `easytab` (update, config, doctor) accessible sans son chemin complet.
$__easytab_bin = Split-Path -Parent $__easytab_term
if ($__easytab_bin -and -not (($env:PATH -split [IO.Path]::PathSeparator) -contains $__easytab_bin)) {
    $env:PATH = $__easytab_bin + [IO.Path]::PathSeparator + $env:PATH
}

# 1. Hors d'EasyTab : relance ce PowerShell sous le wrapper PTY, sauf s'il
# execute une commande ou un script (-Command, -File sans -NoExit).
if (-not $env:EASYTAB_TERM -and -not $env:EASYTAB_DISABLE -and $Host.Name -eq 'ConsoleHost' `
        -and -not [Console]::IsInputRedirected -and -not [Console]::IsOutputRedirected `
        -and (Test-Path -LiteralPath $__easytab_term)) {
    $__easytab_args = [Environment]::GetCommandLineArgs() | Select-Object -Skip 1
    $__easytab_noexit = $__easytab_args -match '^-noe(xit)?$'
    $__easytab_batch = $__easytab_args -match '^-(c|command|f|file|e|ec|encodedcommand|noni|noninteractive)$'
    if ($__easytab_noexit -or -not $__easytab_batch) {
        & $__easytab_term --shell (Get-Process -Id $PID).Path -- -NoLogo
        # Ferme ce PowerShell avec le code d'EasyTab. `exit` ne quitterait que
        # le profil (ou ce fichier) : la fenetre resterait ouverte sur un
        # second PowerShell.
        [Environment]::Exit($LASTEXITCODE)
    }
}

# 2. Sous EasyTab : emet les marqueurs de prompt (OSC 133) et le dossier courant (OSC 7).
# PowerShell n'a pas de hook avant l'execution : easytab-term detecte la touche Entree.
if ($env:EASYTAB_TERM -and -not "$function:prompt".Contains('__easytab_marker')) {
    $function:global:__easytab_original_prompt = $function:prompt

    function global:prompt {
        $status = if ($?) { 0 } else { 1 }
        $e = [char]27
        $a = [char]7
        $__easytab_marker = "$e]133;D;$status$a"
        $location = $ExecutionContext.SessionState.Path.CurrentLocation
        if ($location.Provider.Name -eq 'FileSystem') {
            $path = $location.ProviderPath -replace '\\', '/'
            if (-not $path.StartsWith('/')) { $path = '/' + $path }
            $__easytab_marker += "$e]7;file://$([Environment]::MachineName)$($path -replace ' ', '%20')$a"
        }
        $__easytab_marker + "$e]133;A$a" + (__easytab_original_prompt) + "$e]133;B$a"
    }
}
