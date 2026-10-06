# Guide d'EasyTab

Tout ce que le [README](../README.fr.md) ne dit pas : la liste, les réglages, d'où viennent les
suggestions, les specs personnelles, l'installation depuis les sources.

- [La liste](#la-liste)
- [Réglages](#réglages)
- [D'où viennent les suggestions](#doù-viennent-les-suggestions)
- [Specs personnelles](#specs-personnelles)
- [Workflows](#workflows)
- [Installer](#installer)
- [Compiler depuis les sources](#compiler-depuis-les-sources)
- [Organisation du code](#organisation-du-code)

## La liste

| Touche | Action |
|---|---|
| ↑ / ↓, Maj+Tab | Choisir une suggestion (Maj+Tab remonte) |
| Tab | Insérer la suggestion choisie |
| Entrée | Insérer la suggestion surlignée quand elle complète le mot tapé, ou après ↑ / ↓ ; sinon la commande part comme d'habitude |
| Échap | Fermer la liste jusqu'à la prochaine frappe |
| Ctrl+Espace | Rouvrir la liste fermée avec Échap |
| → | Accepter la suggestion en gris |
| Ctrl+R | Chercher dans tout l'historique (Ctrl+R de nouveau : revenir à la liste habituelle) |

La suggestion en gris, après le curseur, est la commande la plus récente de l'historique qui
prolonge la ligne. Elle s'affiche dans le terminal, même avec la fenêtre flottante, et seulement
quand le curseur est en fin de ligne ; → l'accepte, sinon → déplace le curseur comme d'habitude.
Si le shell affiche déjà sa propre suggestion (prédictions de PowerShell), EasyTab n'ajoute pas
la sienne.

**Recherche (Ctrl+R).** La ligne tapée devient une recherche dans tout l'historique : les
commandes qui contiennent chacun de ses mots, dans n'importe quel ordre et sans tenir compte de la
casse, les plus récentes d'abord (`dock up` trouve `docker compose up -d`). La description donne le
dossier où la commande a été lancée, son échec éventuel, sa durée et sa date. Tab ou Entrée
remplace la ligne par la commande choisie, sans la lancer ; Échap ou Ctrl+R sort de la recherche.
Les workflows qui correspondent passent devant. `search = false` dans `[keys]` laisse Ctrl+R au
shell.

**Correction.** Quand une commande échoue sur une faute de frappe, la commande corrigée s'affiche
en gris au prompt suivant, et → l'accepte : `gti status` → `git status` (commande introuvable,
code 127 en bash et zsh, tout échec dans PowerShell), `git stauts` → `git status` (sous-commande
inconnue d'une spec). Une faute, c'est une lettre en trop, en moins, changée, ou deux lettres
inversées (deux fautes au plus pour un mot de plus de quatre lettres) ; à égalité, le nom le plus
utilisé gagne. Rien n'est proposé pour une ligne avec `|`, `;`, `&` ou `$`. `correct = false`
dans `[list]` le désactive.

**Commande suivante.** Après une commande réussie, la suite habituelle s'affiche en gris au
prompt suivant, et → l'accepte : `git push` après `git commit`, `git push origin v1.2` après
`git tag v1.2` (ou `git tag -a v1.2 …`), `git push -u origin nom` après `git switch -c nom` ou
`git checkout -b nom`. `next = false` dans `[list]` le désactive.

Quand la liste est fermée, Tab, Maj+Tab et Ctrl+Espace gardent leur comportement habituel
(complétion du shell), sauf Ctrl+Espace juste après Échap.

La liste s'affiche dans une fenêtre flottante (`easytab-overlay`), placée sous le
curseur du terminal : coins arrondis, ombre, icônes par type (branche git, script npm, dossier,
fichier, option…) et description en bas. La fenêtre ne prend jamais le focus : le clavier reste
au terminal. Si elle ne trouve pas le curseur (ou avec `EASYTAB_OVERLAY=0`), la liste est
dessinée dans le terminal. Pour comprendre un mauvais placement, `EASYTAB_OVERLAY_LOG=fichier`
journalise chaque position du curseur lue par la fenêtre.

- Windows : Windows Terminal et VS Code, curseur lu par UI Automation.
- macOS : curseur lu par l'accessibilité si le terminal y a accès (Réglages Système >
  Confidentialité et sécurité > Accessibilité : Terminal, iTerm…, dont `easytab-overlay` hérite) ;
  sinon, position déduite de la fenêtre du terminal et de sa taille en colonnes et lignes.
- Linux : sessions X11 seulement, position déduite de la fenêtre active. Sous Wayland, la liste
  reste dans le terminal (`EASYTAB_OVERLAY=1` essaie quand même, pour un terminal XWayland).

Dans le terminal, la liste montre une pastille colorée par type (`>`
commande, `$` sous-commande, `-` option, `@` valeur calculée, `/` dossier…), les lettres tapées en
gras, les arguments attendus en gris (`--cleanup <mode>`) et la description de la suggestion
choisie en bas. Avec `EASYTAB_ICONS=emoji`, les pastilles deviennent des emoji (📦 🚩 🌿 📁…).

## Réglages

`easytab config` crée `~/.easytab/config.toml`, avec chaque réglage commenté ; `easytab doctor`
signale une erreur dans ce fichier. Rouvrir les terminaux après un changement.

```toml
[list]
rows = 8            # suggestions visibles (de 3 à 20)
overlay = true      # fenêtre flottante, ou false pour la liste dans le terminal
theme = "dark"      # ou "light"
icons = "badges"    # ou "emoji" (liste dans le terminal)
history = true      # proposer les commandes déjà tapées
inline = true       # suggestion en gris après le curseur, acceptée avec →
shell = true        # commandes sans spec : complétions de bash-completion ou fish
help = true         # commandes sans spec : options lues dans « commande --help »
correct = true      # après une faute de frappe, commande corrigée en gris au prompt suivant
next = true         # après git commit, git push en gris au prompt suivant (et git tag…)

[keys]
enter_inserts = true  # false : Entrée lance toujours la commande, seul Tab insère
search = true         # Ctrl+R cherche dans l'historique ; false : Ctrl+R reste au shell
```

Les variables `EASYTAB_OVERLAY` et `EASYTAB_ICONS` passent avant le fichier.

Langue des messages (commande `easytab`, scripts d'installation, modèle de `config.toml`) :
anglais par défaut, français si le système l'est (`LC_ALL`, `LC_MESSAGES` ou `LANG` commençant
par `fr` ; sans ces variables, la langue de Windows ou de macOS). `EASYTAB_LANG=fr` ou
`EASYTAB_LANG=en` force le choix.

Désactiver : `EASYTAB_DISABLE=1` pour une session, `easytab uninstall` pour de bon.

## D'où viennent les suggestions

**Specs de Fig.** Plus de 700 commandes (git, docker, npm, symfony, gradle…) sont décrites par
les specs de [withfig/autocomplete](https://github.com/withfig/autocomplete), embarquées dans
EasyTab (voir [specs/README.md](../specs/README.md)).

**Outils de Windows.** Sous Windows, `specs/windows.json` décrit les outils absents des specs
Fig : winget, wsl, choco, scoop, ipconfig, netsh, robocopy, taskkill, tasklist, sc, dism, sfc,
where, findstr, xcopy, icacls, schtasks, shutdown, systeminfo, nslookup, ping, tracert et net.
Elles passent avant celles de Fig (dont `ping` et `where` décrivent les versions Unix). Les
options en `/` sont complétées sans tenir compte de la casse (`/mir` vaut `/MIR`), avec leur
valeur collée (`/LOG:journal.txt`, `/FeatureName:…`).

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

### Specs personnelles

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

## Workflows

Des commandes enregistrées avec des champs à remplir, écrits entre accolades, dans
`~/.easytab/workflows.toml` (`easytab workflows` le crée avec des exemples) :

```toml
[[workflow]]
name = "Shell dans un conteneur"
command = "docker exec -it {conteneur} bash"

[[workflow]]
name = "Branche à partir de main"
command = "git switch -c {branche} main"
description = "Nouvelle branche, partie de main"   # facultatif
```

Un workflow est proposé dans la liste (icône ▸) dès qu'on tape le début de sa commande
(`docker ex`), et dans la recherche Ctrl+R par son nom ou sa commande. Le choisir écrit la
commande jusqu'au premier champ ; la suite s'affiche en gris (`{conteneur} bash`). On tape la
valeur du champ, avec l'aide de la liste habituelle (ici les conteneurs de docker), puis → ajoute
la suite jusqu'au champ suivant. `${HOME}` et `{a,b}` restent du texte pour le shell. Le fichier
est lu à l'ouverture du terminal ; `easytab doctor` y signale une erreur.

## Installer

Le script d'installation copie `easytab`, `easytab-term` et `easytab-overlay` dans
`~/.easytab/bin` et ajoute EasyTab à la config du shell, qui met aussi ce dossier dans le PATH.
Relancer la même commande met à jour ; `easytab update` aussi, pour les shells déjà configurés.
Les anciennes versions mises de côté par une mise à jour (`*.old-…` dans `~/.easytab/bin`) sont
effacées au lancement suivant d'un terminal.

Shells pris en charge : zsh, bash et PowerShell (7 et Windows PowerShell 5), et Git Bash sous
Windows, dans Windows Terminal ou le terminal de VS Code. Dans la fenêtre « Git Bash » par défaut
(mintty), qui ne fournit pas de console aux programmes Windows, EasyTab passe par `winpty`, livré
avec Git for Windows ; sans lui, le shell s'y lance sans EasyTab, avec un message.

**Publier une version** : « Run workflow » sur le workflow `Release` dans l'onglet Actions, avec
le nom de la version (`v0.2.0`), ou `git tag v0.2.0 && git push origin v0.2.0`. Le workflow
compile pour Linux, macOS (Intel et Apple Silicon) et Windows, puis publie les archives et les
scripts d'installation.

## Compiler depuis les sources

Prérequis : [Rust](https://rustup.rs) stable. Sous Windows, le toolchain MSVC
(`rustup default stable-msvc`). Sous Linux, la fenêtre flottante demande WebKitGTK :
`sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev` (Debian, Ubuntu).

```sh
cargo build --release
./target/release/easytab install      # le shell en cours (PowerShell sous Windows, hors Git Bash)
# ouvre un nouveau terminal, puis :
easytab doctor
```

Après une nouvelle compilation, relance `./target/release/easytab install` pour mettre à jour la
copie installée. Les terminaux déjà ouverts gardent l'ancienne version jusqu'à leur fermeture.

Pour voir ce qu'EasyTab détecte pendant la frappe :

```sh
EASYTAB_LOG=/tmp/easytab.log zsh      # puis, dans un autre terminal :
tail -f /tmp/easytab.log
```

Avant chaque push, comme la CI (Linux, macOS et Windows) :

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

## Organisation du code

| Dossier | Rôle |
|---|---|
| `crates/easytab-core` | Suit l'état du shell (marqueurs de prompt `OSC 133`, dossier courant `OSC 7`, copie de l'écran) et transforme la ligne en cours en suggestions à partir des specs. |
| `crates/easytab-term` | Wrapper PTY : lance le shell dans un pseudo-terminal, relaie clavier et écran, dessine la liste de suggestions. |
| `crates/easytab-overlay` | Fenêtre flottante, placée sous le curseur du terminal. |
| `crates/easytab-cli` | Commande `easytab` : `install`, `uninstall`, `update`, `config`, `doctor`, `init`. |
| `shell-integration/` | Scripts zsh, bash et PowerShell qui relancent le shell sous `easytab-term` et émettent les marqueurs de prompt. |
| `specs/` | Specs de complétion importées de Fig, et celles des outils de Windows (`windows.json`). |
| `tools/` | Script d'import des specs Fig. |

L'architecture et ses choix sont décrits dans [architecture.md](architecture.md).
