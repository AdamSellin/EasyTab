[English](../en/suggestions.md) · **Français**

# D'où viennent les suggestions

**Specs de Fig.** Plus de 700 commandes (git, docker, npm, symfony, gradle…) sont décrites par
les specs de [withfig/autocomplete](https://github.com/withfig/autocomplete), embarquées dans
EasyTab (voir [specs/README.md](../../../specs/README.md)).

**Outils de Windows.** Sous Windows, `specs/windows.json` décrit les outils absents des specs
Fig : winget, wsl, choco, scoop, ipconfig, netsh, robocopy, taskkill, tasklist, sc, dism, sfc,
where, findstr, xcopy, icacls, schtasks, shutdown, systeminfo, nslookup, ping, tracert et net.
Elles passent avant celles de Fig (dont `ping` et `where` décrivent les versions Unix). Les
options en `/` sont complétées sans tenir compte de la casse (`/mir` vaut `/MIR`), avec leur
valeur collée (`/LOG:journal.txt`, `/FeatureName:…`).

**EasyTab lui-même.** `easytab` complète ses propres sous-commandes et options (`easytab install
--shell pwsh`…), décrites dans `specs/easytab.json`.

**Historique.** Les commandes déjà tapées qui prolongent la ligne en cours passent
en tête (`docker-compose u` → `docker-compose up -d --build`, icône d'horloge), tirées de
l'historique du shell (`~/.bash_history`, `~/.zsh_history`, historique PSReadLine de PowerShell).
EasyTab retient en plus, dans `~/.easytab/history.jsonl`, le dossier, le code de sortie et la
durée de chaque commande lancée sous lui : celles lancées dans le dossier courant passent devant,
celles dont la dernière exécution a échoué derrière. `history = false` désactive les deux.

**Variables d'environnement.** `$HO` propose `$HOME`, `$HOSTNAME`… avec leur valeur ; `${HO`
donne `${HOME}`. Sous PowerShell, c'est `$env:PA` → `$env:PATH`. Ce sont les variables connues à
l'ouverture du terminal : une variable exportée plus tard dans la session n'y est pas.

**Alias.** Sous bash et zsh, l'intégration envoie les alias du shell à EasyTab au premier prompt,
puis quand ils changent (séquence `OSC 6973`, ignorée par les terminaux). Avec `alias g=git`,
`g pu` se complète comme `git pu`, et les alias sont proposés comme commandes, avec leur valeur
en description. Les alias de PowerShell viennent de `Get-Command`.

**Classement.** Les suggestions sont classées par qualité de correspondance (début du nom, puis
recherche floue : `git chk` trouve `checkout`), puis par fréquence d'utilisation, retenue dans
`~/.easytab/usage.json`. La recherche floue ne sert que si aucune suggestion ne commence par
le mot tapé ni ne le contient : `git che` propose `checkout`, pas `credential-helper-selector`.

**Valeurs calculées.** Les branches, scripts, conteneurs… viennent des *generators* des specs
Fig : EasyTab lance la commande prévue par la spec (`git branch`, lecture de `package.json`…) en
arrière-plan, passe sa sortie au JavaScript de la spec dans un moteur JS embarqué (QuickJS), puis
complète la liste dès que le résultat arrive. La frappe n'attend jamais. Certaines specs calculent
ainsi une partie de leurs sous-commandes (`generateSpec` de Fig : les commandes de `composer`,
celles de `php bin/console` dans un projet Symfony…) ; le résultat est gardé une minute par
dossier.

**Fichiers du projet.** Quelques valeurs sont lues directement dans les fichiers, sans lancer de
commande (donc aussi sous Windows sans `bash`) :

- les cibles du `Makefile` pour `make` (avec leur commentaire `## …` comme description) ;
- les scripts de `composer.json` pour `composer run-script` / `composer run` ;
- les projets de `angular.json` pour `ng build`, `ng serve`, `ng test`… ;
- les scripts de `package.json` (cherché aussi dans les dossiers parents) pour `npm run`,
  `yarn` / `yarn run`, `pnpm` / `pnpm run` et `bun run` ;
- les hôtes SSH pour `ssh`, `sftp` et `scp` (`hôte:`) : lignes `Host` de `~/.ssh/config` (et des
  fichiers de ses `Include`, hors motifs `*`) et hôtes de `~/.ssh/known_hosts` (hors entrées
  hachées). Après `user@`, seul l'hôte est complété.

Les fichiers sont relus quand ils changent. Les services de `docker compose` viennent des specs
Fig.

**PowerShell.** Les commandes PowerShell (`Get-ChildItem`…) et leurs paramètres viennent de
PowerShell lui-même (`Get-Command`), lancé une fois en arrière-plan. Les alias (`ls`, `gci`,
`cat`…) sont proposés avec la commande qu'ils désignent et en reçoivent les paramètres
(`ls -Recurse`) : dans PowerShell, `ls` est `Get-ChildItem`, pas le `ls` d'Unix. Un alias vers
un programme (`g` → `git`) reçoit sa spec.

**Complétions du shell.** Pour une commande sans spec, EasyTab demande à fish puis à
bash-completion, s'ils sont installés, ce qu'ils proposeraient après Tab : `fish -c 'complete -C …'`
(avec les descriptions), puis la fonction de complétion que bash-completion déclare pour la
commande (y compris celles de l'utilisateur, dans `~/.local/share/bash-completion/completions`).
Ils sont lancés en arrière-plan, sans la configuration de l'utilisateur, 2 secondes au plus ; la
réponse est gardée par dossier et par ligne. Une commande qu'aucun des deux ne connaît passe au
`--help`. Sous Windows, seulement dans Git Bash ; jamais dans PowerShell, qui décrit ses commandes
lui-même. `shell = false` dans les réglages le désactive.

**`--help`.** Pour une commande installée qui n'a pas de spec ni de complétion du shell, EasyTab
lance une fois `commande --help` en arrière-plan (3 secondes au plus, depuis le dossier temporaire)
et en tire ses options (`-x, --option=VALEUR  description`). La réponse est gardée dans
`~/.easytab/cache/help.json` tant que le programme ne change pas. Seules les commandes du PATH sont
interrogées, jamais `rm`, `dd`, `shutdown`, `reboot`, `halt`, `poweroff`, `mkfs`, `format`, ni les
scripts `.bat` / `.cmd` sous Windows ; une sortie qui ne ressemble pas à une aide (code de sortie
autre que 0 ou 1, moins de deux options) est ignorée. `help = false` dans les réglages le désactive.

## Specs personnelles

Les fichiers `~/.easytab/specs/*.json` décrivent des commandes en plus de celles de Fig, ou à
leur place : une spec de même nom remplace la spec embarquée. Une commande décrite ainsi est
proposée même si elle n'est pas dans le PATH (fonction ou alias du shell). Les fichiers sont lus
à l'ouverture du terminal ; une erreur est notée dans le journal (`EASYTAB_LOG`).

Le format est celui des specs embarquées (specs Fig converties) : `names` (toujours un tableau),
`description`, `subcommands`, `options` et `args`. Un argument peut avoir des `suggestions`
(`{"names": [...], "description": ...}`), des `templates` (`"filepaths"`, `"folders"`), être
`optional`, `variadic` ou `is_command` ; une option peut être `persistent` (valable dans les
sous-commandes) ou `requires_equals` (`--opt=valeur`) ; `"load": "git"` reprend une autre spec.
Un fichier contient une spec, ou un tableau de specs.

```json
{
  "names": ["deploy"],
  "description": "Déploie l'application",
  "subcommands": [
    {
      "names": ["app", "a"],
      "description": "Déploie l'application web",
      "args": [{ "name": "env", "suggestions": [{ "names": ["staging"] }, { "names": ["prod"] }] }]
    }
  ],
  "options": [
    { "names": ["-f", "--force"], "description": "Sans confirmation" },
    { "names": ["--config"], "args": [{ "name": "fichier", "templates": ["filepaths"] }] }
  ]
}
```

[← Sommaire](README.md)
