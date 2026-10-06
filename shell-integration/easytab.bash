# Intégration EasyTab pour bash, générée par `easytab init bash`.
# `easytab install` la charge deux fois : en haut du ~/.bashrc, pour relancer le
# shell sous EasyTab avant de lire le reste, et en bas, pour poser les hooks
# après les thèmes de prompt.
# Désactiver ponctuellement : EASYTAB_DISABLE=1 bash

# Rend `easytab` (update, config, doctor) accessible sans son chemin complet.
__easytab_bin=__EASYTAB_TERM_BIN__
if [[ "$__easytab_bin" == */* || "$__easytab_bin" == *\\* ]]; then
  __easytab_bin=${__easytab_bin%[/\\]*}
  # Git Bash : C:\Users\... devient /c/Users/...
  if [[ -n "$MSYSTEM" ]] && command -v cygpath >/dev/null 2>&1; then
    __easytab_bin=$(cygpath -u "$__easytab_bin")
  fi
  [[ ":$PATH:" == *":$__easytab_bin:"* ]] || export PATH="$__easytab_bin:$PATH"
fi
unset __easytab_bin

# 1. Hors d'EasyTab : relance ce shell sous le wrapper PTY.
if [[ -z "$EASYTAB_TERM" && -z "$EASYTAB_DISABLE" && $- == *i* && -t 0 && -t 1 ]] \
    && command -v __EASYTAB_TERM_BIN__ >/dev/null 2>&1; then
  if [[ -n "$MSYSTEM" ]]; then
    # Git Bash : lancé par un programme Windows, bash doit être un shell de
    # connexion pour que /etc/profile remette /usr/bin dans le PATH.
    if [[ "$TERM_PROGRAM" == mintty ]] && command -v winpty >/dev/null 2>&1; then
      # mintty ne donne pas de console aux programmes Windows : winpty (livré
      # avec Git for Windows) leur en fournit une.
      exec winpty __EASYTAB_TERM_BIN__ --shell "${BASH:-bash}" -- -l
    fi
    exec __EASYTAB_TERM_BIN__ --shell "${BASH:-bash}" -- -l
  fi
  exec __EASYTAB_TERM_BIN__ --shell "${BASH:-bash}"
fi

# 2. Sous EasyTab : émet les marqueurs de prompt (OSC 133) et le dossier courant (OSC 7).
# bash n'a pas de hook avant l'exécution : easytab-term détecte la touche Entrée.
if [[ -n "$EASYTAB_TERM" ]]; then
  __easytab_save_status() {
    __easytab_status=$?
  }

  # Envoie les alias à EasyTab (`g push` se complète comme `git push`), au
  # premier prompt puis quand ils changent. Lus dans BASH_ALIASES, sans
  # sous-shell : lancer un programme à chaque prompt ralentirait Git Bash.
  __easytab_aliases_sent=
  __easytab_send_aliases() {
    local all="${!BASH_ALIASES[*]}=${BASH_ALIASES[*]}" name
    [[ "$all" == "$__easytab_aliases_sent" ]] && return
    __easytab_aliases_sent=$all
    printf '\e]6973;aliases\a'
    for name in "${!BASH_ALIASES[@]}"; do
      printf '\e]6973;alias;%s=%s\a' "$name" "${BASH_ALIASES[$name]}"
    done
  }

  __easytab_prompt() {
    __easytab_send_aliases
    printf '\e]133;D;%s\a\e]7;file://%s%s\a\e]133;A\a' "$__easytab_status" "$HOSTNAME" "${PWD// /%20}"
    # Ajouté à chaque prompt, car certains thèmes réécrivent PS1.
    [[ "$PS1" == *'\e]133;B\a'* ]] || PS1="$PS1"'\[\e]133;B\a\]'
    return "$__easytab_status"
  }

  # Premier pour lire le vrai code de sortie, dernier pour passer après les thèmes.
  # Séparés par des retours à la ligne pour supporter un PROMPT_COMMAND finissant par « ; ».
  # Ajoutés seulement s'ils manquent : ce script peut être chargé deux fois.
  if [[ "$PROMPT_COMMAND" != *__easytab_prompt* ]]; then
    __easytab_nl=$'\n'
    PROMPT_COMMAND="__easytab_save_status${__easytab_nl}${PROMPT_COMMAND:+$PROMPT_COMMAND$__easytab_nl}__easytab_prompt"
    unset __easytab_nl
  fi
fi
