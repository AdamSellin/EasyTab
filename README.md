# EasyTab

Autocomplétion graphique pour le terminal (Windows, Linux, macOS), dans l'esprit de Fig :
une liste de suggestions style IDE qui s'affiche pendant que tu tapes une commande.

> **État : premières suggestions.** Une liste s'affiche dans le terminal pendant la frappe,
> pour 47 commandes courantes (git, docker, npm, cargo, kubectl…) et les fichiers et
> dossiers. Voir la suite du plan dans [docs/architecture.md](docs/architecture.md).

## Utilisation

| Touche | Action |
|---|---|
| ↑ / ↓ | Choisir une suggestion |
| Tab | Insérer la suggestion choisie |
| Échap | Fermer la liste jusqu'à la prochaine frappe |

Quand la liste est fermée, Tab garde son comportement habituel (complétion du shell).

## Organisation

| Dossier | Rôle |
|---|---|
| `crates/easytab-core` | Suit l'état du shell (marqueurs de prompt `OSC 133`, dossier courant `OSC 7`, copie de l'écran) et transforme la ligne en cours en suggestions à partir des specs. |
| `crates/easytab-term` | Wrapper PTY : lance le shell dans un pseudo-terminal, relaie clavier et écran, dessine la liste de suggestions. |
| `crates/easytab-cli` | Commande `easytab` : `install`, `uninstall`, `doctor`, `init`. |
| `shell-integration/` | Scripts zsh et bash qui relancent le shell sous `easytab-term` et émettent les marqueurs de prompt. |
| `specs/` | Specs de complétion importées de Fig (voir [specs/README.md](specs/README.md)). |
| `tools/` | Script d'import des specs Fig. |

## Essayer

Prérequis : [Rust](https://rustup.rs) stable. Shells pris en charge pour l'instant : zsh et bash
(Linux, macOS), et Git Bash sous Windows dans Windows Terminal ou le terminal de VS Code. La fenêtre
« Git Bash » par défaut (mintty) n'est pas encore prise en charge : le shell s'y lance sans EasyTab, avec un message.
Sous Windows, utilise le toolchain Rust MSVC (`rustup default stable-msvc`).

```sh
cargo build --release
./target/release/easytab install      # ajoute un bloc à ~/.zshrc ou ~/.bashrc
# ouvre un nouveau terminal, puis :
./target/release/easytab doctor       # vérifie que tout est actif
```

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
