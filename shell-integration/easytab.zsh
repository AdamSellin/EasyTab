# Intégration EasyTab pour zsh, générée par `easytab init zsh`.
# `easytab install` la charge deux fois : en haut du ~/.zshrc, pour relancer le
# shell sous EasyTab avant de lire le reste, et en bas, pour poser les hooks
# après les thèmes de prompt.
# Désactiver ponctuellement : EASYTAB_DISABLE=1 zsh

# 1. Hors d'EasyTab : relance ce shell sous le wrapper PTY.
if [[ -z "$EASYTAB_TERM" && -z "$EASYTAB_DISABLE" && -o interactive && -t 0 && -t 1 ]] \
    && command -v __EASYTAB_TERM_BIN__ >/dev/null 2>&1; then
  exec __EASYTAB_TERM_BIN__ --shell "${commands[zsh]:-zsh}"
fi

# 2. Sous EasyTab : émet les marqueurs de prompt (OSC 133) et le dossier courant (OSC 7).
if [[ -n "$EASYTAB_TERM" ]]; then
  __easytab_precmd() {
    local code=$?
    print -n "\e]133;D;${code}\a\e]7;file://${HOST}${PWD// /%20}\a\e]133;A\a"
    # Ajouté à chaque prompt, car certains thèmes réécrivent PS1.
    [[ "$PS1" == *$'\e]133;B\a'* ]] || PS1="${PS1}%{"$'\e]133;B\a'"%}"
  }

  __easytab_preexec() {
    print -n "\e]133;C\a"
  }

  # add-zsh-hook ignore un hook déjà présent : ce script peut être chargé deux fois.
  autoload -Uz add-zsh-hook
  add-zsh-hook precmd __easytab_precmd
  add-zsh-hook preexec __easytab_preexec
fi
