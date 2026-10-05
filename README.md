# EasyTab

Autocomplétion graphique pour le terminal (Windows, Linux, macOS), dans l'esprit de Fig :
une liste de suggestions style IDE qui s'affiche pendant que tu tapes une commande.

> **État : suggestions dynamiques.** Une liste s'affiche dans le terminal pendant la frappe,
> pour plus de 700 commandes (les specs de Fig : git, docker, npm, symfony, gradle…), les fichiers et dossiers,
> et les valeurs propres à ton projet : branches git, scripts npm, conteneurs docker… Voir la
> suite du plan dans [docs/architecture.md](docs/architecture.md).

## Utilisation

| Touche | Action |
|---|---|
| ↑ / ↓ | Choisir une suggestion |
| Tab | Insérer la suggestion choisie |
| Entrée | Après ↑ / ↓ : insérer la suggestion choisie (sans flèche, la commande part comme d'habitude) |
| Échap | Fermer la liste jusqu'à la prochaine frappe |

Quand la liste est fermée, Tab garde son comportement habituel (complétion du shell).

La liste s'affiche dans une fenêtre flottante façon Fig (`easytab-overlay`), placée sous le
curseur du terminal : coins arrondis, ombre, icônes par type (branche
git, script npm, dossier, fichier, option…) et description en bas. La fenêtre ne prend jamais le
focus : le clavier reste au terminal. Si elle ne trouve pas le curseur (ou avec
`EASYTAB_OVERLAY=0`), la liste est dessinée dans le terminal. Pour comprendre un mauvais placement,
`EASYTAB_OVERLAY_LOG=fichier` journalise chaque position du curseur lue par la fenêtre.

- Windows : Windows Terminal et VS Code, curseur lu par UI Automation.
- macOS : curseur lu par l'accessibilité si le terminal y a accès (Réglages Système >
  Confidentialité et sécurité > Accessibilité : Terminal, iTerm…, dont `easytab-overlay` hérite) ;
  sinon, position déduite de la fenêtre du terminal et de sa taille en colonnes et lignes.
- Linux : sessions X11 seulement, position déduite de la fenêtre active. Sous Wayland, la liste
  reste dans le terminal (`EASYTAB_OVERLAY=1` essaie quand même, pour un terminal XWayland).

Dans le terminal, la liste ressemble aussi à celle de Fig : une pastille colorée par type (`>` commande, `$` sous-commande,
`-` option, `@` valeur calculée, `/` dossier…), les lettres tapées en gras, les arguments attendus
en gris (`--cleanup <mode>`) et la description de la suggestion choisie en bas. Avec
`EASYTAB_ICONS=emoji`, les pastilles deviennent des emoji (📦 🚩 🌿 📁…).

### Réglages

`easytab config` crée `~/.easytab/config.toml`, avec chaque réglage commenté ; `easytab doctor`
signale une erreur dans ce fichier. Rouvrir les terminaux après un changement.

```toml
[list]
rows = 8            # suggestions visibles (de 3 à 20)
overlay = true      # fenêtre flottante, ou false pour la liste dans le terminal
theme = "dark"      # ou "light"
icons = "badges"    # ou "emoji" (liste dans le terminal)
history = true      # proposer les commandes déjà tapées

[keys]
enter_inserts = true  # false : Entrée lance toujours la commande, seul Tab insère
```

Les variables `EASYTAB_OVERLAY` et `EASYTAB_ICONS` passent avant le fichier.

Comme dans Fig, les commandes déjà tapées qui prolongent la ligne en cours passent en tête
(`docker-compose u` → `docker-compose up -d --build`, icône d'horloge), tirées de l'historique du
shell (`~/.bash_history`, `~/.zsh_history`, historique PSReadLine de PowerShell).

Les suggestions sont classées par qualité de correspondance (début du nom, puis recherche floue :
`git chk` trouve `checkout`), puis par fréquence d'utilisation, retenue dans `~/.easytab/usage.json`.

Les suggestions dynamiques (branches, scripts…) viennent des *generators* des specs Fig : EasyTab
lance la commande prévue par la spec (`git branch`, lecture de `package.json`…) en arrière-plan,
passe sa sortie au JavaScript de la spec dans un moteur JS embarqué (QuickJS), puis complète la
liste dès que le résultat arrive. La frappe n'attend jamais. Certaines specs calculent ainsi une
partie de leurs sous-commandes (`generateSpec` de Fig : les commandes de `composer`, celles de
`php bin/console` dans un projet Symfony…) ; le résultat est gardé une minute par dossier.

## Organisation

| Dossier | Rôle |
|---|---|
| `crates/easytab-core` | Suit l'état du shell (marqueurs de prompt `OSC 133`, dossier courant `OSC 7`, copie de l'écran) et transforme la ligne en cours en suggestions à partir des specs. |
| `crates/easytab-term` | Wrapper PTY : lance le shell dans un pseudo-terminal, relaie clavier et écran, dessine la liste de suggestions. |
| `crates/easytab-cli` | Commande `easytab` : `install`, `uninstall`, `update`, `config`, `doctor`, `init`. |
| `shell-integration/` | Scripts zsh, bash et PowerShell qui relancent le shell sous `easytab-term` et émettent les marqueurs de prompt. |
| `specs/` | Specs de complétion importées de Fig (voir [specs/README.md](specs/README.md)). |
| `tools/` | Script d'import des specs Fig. |

## Installer

Sans Rust ni compilation, depuis la dernière [version publiée](https://github.com/AdamSellin/EasyTab/releases) :

```sh
# Linux, macOS
curl -fsSL https://github.com/AdamSellin/EasyTab/releases/latest/download/install.sh | sh
```

```powershell
# Windows (PowerShell et Git Bash, dans Windows Terminal ou VS Code)
irm https://github.com/AdamSellin/EasyTab/releases/latest/download/install.ps1 | iex
```

Ensuite, `easytab update` installe la dernière version publiée, pour les shells déjà configurés
(dépôt privé : avec les identifiants GitHub de git, ou `GITHUB_TOKEN`). Les anciennes versions
mises de côté par une mise à jour (`*.old-…` dans `~/.easytab/bin`) sont effacées au lancement
suivant d'un terminal.

Tant que le dépôt est privé, ces liens répondent 404 sans connexion à GitHub. Lance alors le
script depuis un clone du dépôt : il télécharge la version publiée avec les identifiants GitHub
que git utilise déjà (ou `GITHUB_TOKEN`, ou GitHub CLI).

```sh
sh scripts/install.sh          # Linux, macOS
```

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install.ps1   # Windows
```

Le script copie `easytab` et `easytab-term` dans `~/.easytab/bin` et ajoute EasyTab à la config
du shell. Relancer la même commande met à jour. Ouvre ensuite un nouveau terminal.

Publier une version : `git tag v0.2.0 && git push origin v0.2.0`, ou « Run workflow » sur le
workflow `Release` dans l'onglet Actions, avec le nom de la version. Le workflow `Release`
compile pour Linux, macOS (Intel et Apple Silicon) et Windows, puis publie les archives et les
scripts d'installation.

## Compiler depuis les sources

Prérequis : [Rust](https://rustup.rs) stable. Shells pris en charge : zsh, bash et PowerShell
(`easytab install --shell pwsh`, PowerShell 7 et Windows PowerShell 5), et Git Bash sous Windows,
dans Windows Terminal ou le terminal de VS Code. Dans la fenêtre « Git Bash » par défaut (mintty),
qui ne fournit pas de console aux programmes Windows, EasyTab passe par `winpty`, livré avec Git
for Windows ; sans lui, le shell s'y lance sans EasyTab, avec un message.
Sous Windows, utilise le toolchain Rust MSVC (`rustup default stable-msvc`). Sous Linux, la
fenêtre flottante demande WebKitGTK : `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev`
(Debian, Ubuntu).

```sh
cargo build --release
./target/release/easytab install      # copie les programmes dans ~/.easytab/bin et ajoute un bloc à ~/.zshrc ou ~/.bashrc
# ouvre un nouveau terminal, puis :
easytab doctor                        # ou ~/.easytab/bin/easytab doctor
```

Après une nouvelle compilation, relance `./target/release/easytab install` pour mettre à jour la
copie installée. Les terminaux déjà ouverts gardent l'ancienne version jusqu'à leur fermeture
(sous Windows, elle est mise de côté puis effacée à l'installation suivante).

Pour voir ce qu'EasyTab détecte pendant la frappe :

```sh
EASYTAB_LOG=/tmp/easytab.log zsh      # puis, dans un autre terminal :
tail -f /tmp/easytab.log
```

Désactiver : `EASYTAB_DISABLE=1` pour une session, `easytab uninstall` pour de bon.

## Développement

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

La CI GitHub Actions lance ces vérifications sur Linux, macOS et Windows.

## Licence

MIT
