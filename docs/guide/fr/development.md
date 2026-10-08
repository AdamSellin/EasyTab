[English](../en/development.md) · **Français**

# Développement

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
cargo deny check advisories licenses sources   # cargo install cargo-deny
```

Rust 1.88 au minimum (`rust-version` dans `Cargo.toml`, vérifié par la CI).

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

L'architecture et ses choix sont décrits dans [architecture.md](../../architecture.md).

## Publier une version

`main` est protégée : tout passe par une PR. Pour publier la version `0.2.0` :

1. une PR « Version 0.2.0 » qui passe `version` à `0.2.0` dans `Cargo.toml` (puis
   `cargo update --workspace` pour `Cargo.lock`) ;
2. une fois la PR fusionnée, « Run workflow » sur le workflow `Release` dans l'onglet Actions,
   avec le nom de la version (`v0.2.0`).

Le workflow compile pour Linux, macOS (Intel et Apple Silicon) et Windows, puis publie les
archives, `SHA256SUMS` et les scripts d'installation. Les notes de version sont tirées des titres
des PR. Les tags `v*` ne peuvent être ni déplacés ni supprimés.

### Signature (Windows)

Les versions ne sont pas signées. SignPath Foundation (signature
gratuite pour l'open source) a refusé le projet en octobre 2026, trop récent et pas encore assez
connu ; une nouvelle demande est possible plus tard, sur [signpath.org](https://signpath.org).
Le workflow est prêt : il fait signer par SignPath (Foundation ou abonnement payant) les trois
`.exe`, `easytab-profile.ps1` et `install.ps1` dès que le dépôt a la variable
`SIGNPATH_ORGANIZATION_ID` ; sans elle, la version sort non signée. À régler une fois :

- dans SignPath : activer le système de compilation de confiance « GitHub.com » et le lier au
  projet ; configuration d'artefact du projet : `.signpath/artifact-configuration.xml` ;
- dans GitHub (Settings → Secrets and variables → Actions) : le secret `SIGNPATH_API_TOKEN`
  (jeton d'un utilisateur SignPath qui peut soumettre), et les variables
  `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG` et `SIGNPATH_POLICY_SLUG`.

Le workflow vérifie chaque signature et refuse un script signé qui commencerait par un BOM
(`irm | iex` le refuse).

### winget

`packaging/winget` décrit le paquet `AdamSellin.EasyTab` (le zip, avec `easytab`
dans le PATH ; il reste à lancer `easytab install`). Avant de le proposer, y mettre la version,
l'URL et le SHA-256 (`Get-FileHash`) de la dernière version publiée, puis
`winget validate --manifest packaging\winget` et `wingetcreate submit packaging\winget`.
Ensuite, pour chaque version : `wingetcreate update AdamSellin.EasyTab --version 0.2.0
--urls <url du zip> --submit`.

[← Sommaire](README.md)
