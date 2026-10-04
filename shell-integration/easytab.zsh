# Intégration EasyTab pour zsh, générée par `easytab init zsh`.
# Désactiver ponctuellement : EASYTAB_DISABLE=1 zsh

# 1. Hors d'EasyTab : relance ce shell sous le wrapper PTY.
if [[ -z "$EASYTAB_TERM" && -z "$EASYTAB_DISABLE" && -o interactive && -t 0 && -t 1 ]] \
    && command -v __EASYTAB_TERM_BIN__ >/dev/null 2>&1; then
  exec __EASYTAB_TERM_BIN__ --shell "${commands[zsh]:-zsh}"
fi

# 2. Sous EasyTab : émet les marqueurs de prompt OSC 133.
if [[ -n "$EASYTAB_TERM" && -z "$__easytab_loaded" ]]; then
  __easytab_loaded=1

  __easytab_precmd() {
    local code=$?
    print -n "\e]133;D;${code}\a\e]133;A\a"
    # Ajouté à chaque prompt, car certains thèmes réécrivent PS1.
    [[ "$PS1" == *$'\e]133;B\a'* ]] || PS1="${PS1}%{"$'\e]133;B\a'"%}"
  }

  __easytab_preexec() {
    print -n "\e]133;C\a"
  }

  autoload -Uz add-zsh-hook
  add-zsh-hook precmd __easytab_precmd
  add-zsh-hook preexec __easytab_preexec
fi
