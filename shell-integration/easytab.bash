# Intégration EasyTab pour bash, générée par `easytab init bash`.
# Désactiver ponctuellement : EASYTAB_DISABLE=1 bash

# 1. Hors d'EasyTab : relance ce shell sous le wrapper PTY.
if [[ -z "$EASYTAB_TERM" && -z "$EASYTAB_DISABLE" && $- == *i* && -t 0 && -t 1 ]] \
    && command -v __EASYTAB_TERM_BIN__ >/dev/null 2>&1; then
  exec __EASYTAB_TERM_BIN__ --shell "${BASH:-bash}"
fi

# 2. Sous EasyTab : émet les marqueurs de prompt OSC 133.
# bash n'a pas de hook avant l'exécution : easytab-term détecte la touche Entrée.
if [[ -n "$EASYTAB_TERM" && -z "$__easytab_loaded" ]]; then
  __easytab_loaded=1

  __easytab_save_status() {
    __easytab_status=$?
  }

  __easytab_prompt() {
    printf '\e]133;D;%s\a\e]133;A\a' "$__easytab_status"
    # Ajouté à chaque prompt, car certains thèmes réécrivent PS1.
    [[ "$PS1" == *'\e]133;B\a'* ]] || PS1="$PS1"'\[\e]133;B\a\]'
    return "$__easytab_status"
  }

  # Premier pour lire le vrai code de sortie, dernier pour passer après les thèmes.
  # Séparés par des retours à la ligne pour supporter un PROMPT_COMMAND finissant par « ; ».
  __easytab_nl=$'\n'
  PROMPT_COMMAND="__easytab_save_status${__easytab_nl}${PROMPT_COMMAND:+$PROMPT_COMMAND$__easytab_nl}__easytab_prompt"
  unset __easytab_nl
fi
