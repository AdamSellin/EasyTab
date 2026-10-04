# EasyTab

Autocomplétion graphique pour le terminal (Windows, Linux, macOS), dans l'esprit de Fig :
une liste de suggestions style IDE qui s'affiche pendant que tu tapes une commande.

> **État : squelette.** Le wrapper PTY tourne et sait déjà quelle commande est en cours
> de saisie, mais n'affiche pas encore de suggestions. Voir le plan dans
> [docs/architecture.md](docs/architecture.md).

## Organisation

| Dossier | Rôle |
|---|---|
| `crates/easytab-core` | Suit l'état du shell (marqueurs de prompt `OSC 133` + copie de l'écran) et en déduit la ligne en cours. Accueillera le moteur de suggestions. |
| `crates/easytab-term` | Wrapper PTY : lance le shell dans un pseudo-terminal et relaie clavier et écran. |
| `crates/easytab-cli` | Commande `easytab` : `install`, `uninstall`, `doctor`, `init`. |
| `shell-integration/` | Scripts zsh et bash qui relancent le shell sous `easytab-term` et émettent les marqueurs de prompt. |

## Essayer

Prérequis : [Rust](https://rustup.rs) stable. Shells pris en charge pour l'instant : zsh et bash (Linux, macOS).

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
