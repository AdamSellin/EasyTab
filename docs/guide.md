# Guide d'EasyTab

Tout ce que le [README](../README.fr.md) ne dit pas : la liste, les réglages, d'où viennent les
suggestions, les specs personnelles, l'installation depuis les sources.

- [La liste](#la-liste)
- [Réglages](#réglages)
- [D'où viennent les suggestions](#doù-viennent-les-suggestions)
- [Specs personnelles](#specs-personnelles)
- [Installer](#installer)
- [Compiler depuis les sources](#compiler-depuis-les-sources)
- [Organisation du code](#organisation-du-code)

## La liste

| Touche | Action |
|---|---|
| ↑ / ↓ | Choisir une suggestion |
| Tab | Insérer la suggestion choisie |
| Entrée | Insérer la suggestion surlignée quand elle complète le mot tapé, ou après ↑ / ↓ ; sinon la commande part comme d'habitude |
| Échap | Fermer la liste jusqu'à la prochaine frappe |

Quand la liste est fermée, Tab garde son comportement habituel (complétion du shell).

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
help = true         # commandes sans spec : options lues dans « commande --help »

[keys]
enter_inserts = true  # false : Entrée lance toujours la commande, seul Tab insère
```

Les variables `EASYTAB_OVERLAY` et `EASYTAB_ICONS` passent avant le fichier.

Désactiver : `EASYTAB_DISABLE=1` pour une session, `easytab uninstall` pour de bon.

## D'où viennent les suggestions

**Specs de Fig.** Plus de 700 commandes (git, docker, npm, symfony, gradle…) sont décrites par
les specs de [withfig/autocomplete](https://github.com/withfig/autocomplete), embarquées dans
EasyTab (voir [specs/README.md](../specs/README.md)).

**Historique.** Les commandes déjà tapées qui prolongent la ligne en cours passent
en tête (`docker-compose u` → `docker-compose up -d --build`, icône d'horloge), tirées de
l'historique du shell (`~/.bash_history`, `~/.zsh_history`, historique PSReadLine de PowerShell).

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
PowerShell lui-même (`Get-Command`), lancé une fois en arrière-plan.

**`--help`.** Pour une commande installée qui n'a pas de spec, EasyTab lance une fois
`commande --help` en arrière-plan (3 secondes au plus, depuis le dossier temporaire) et en tire
ses options (`-x, --option=VALEUR  description`). La réponse est gardée dans
`~/.easytab/cache/help.json` tant que le programme ne change pas. Seules les commandes du PATH
sont interrogées, jamais `rm`, `dd`, `shutdown`, `reboot`, `halt`, `poweroff`, `mkfs`, `format`,
ni les scripts `.bat` / `.cmd` sous Windows ; une sortie qui ne ressemble pas à une aide (code de
sortie autre que 0 ou 1, moins de deux options) est ignorée. `help = false` dans les réglages le
désactive.

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
./target/release/easytab install      # ajoute --shell pwsh pour PowerShell
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
| `specs/` | Specs de complétion importées de Fig. |
| `tools/` | Script d'import des specs Fig. |

L'architecture et ses choix sont décrits dans [architecture.md](architecture.md).
