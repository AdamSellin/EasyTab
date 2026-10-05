# EasyTab : notes pour Claude

Autocomplétion graphique pour le terminal, façon Fig : un wrapper PTY lit la ligne en cours et
affiche des suggestions tirées des specs de Fig. Vue d'ensemble des dossiers : voir le README.

## Commandes

Avant chaque push, comme la CI (`.github/workflows/ci.yml`) :

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- `crates/easytab-term/tests/e2e.rs` lance vraiment `easytab-term` dans un pseudo-terminal (bash
  sous Linux, PowerShell et Git Bash sous Windows), tape `git checko`, Tab, ↓, Entrée, Échap.
  À compléter quand on touche au clavier, à la liste ou au démarrage du shell.
- Installer en local : `cargo build --release`, puis `target/release/easytab install` (ajouter
  `--shell pwsh` pour PowerShell). Fermer les terminaux ouverts : ils gardent l'ancien
  `easytab-term`.
- La fenêtre flottante (`crates/easytab-overlay`) ne compile que sous Windows. Depuis Linux :
  `cargo clippy -p easytab-overlay --target x86_64-pc-windows-gnu`. `easytab-term` ne se
  compile pas ainsi (QuickJS demande un gcc MinGW).
- Publier une version : « Run workflow » sur `Release` dans l'onglet Actions, avec le nom de la
  version (`v0.1.1`), ou pousser un tag `v*`.

- Réglages de l'utilisateur : `crates/easytab-core/src/config.rs` (`~/.easytab/config.toml`),
  partagé avec `easytab-cli` par `#[path]` pour ne pas y embarquer QuickJS. Tout nouveau réglage
  va aussi dans `TEMPLATE` et dans le README.

## Conventions

- Code, commentaires, messages, commits et PR en français.
- Une PR par sujet, fusionnée en squash. Tests unitaires dans le fichier du code (`mod tests`).
- `shell-integration/easytab.ps1` et les chaînes de `scripts/install.ps1` restent en ASCII :
  Windows PowerShell 5.1 lit l'UTF-8 sans BOM comme de l'ANSI, et `irm | iex` refuse un BOM.

## Windows, les pièges connus

- Toolchain Rust MSVC (`rustup default stable-msvc`) ; la GNU échoue sur `dlltool`.
- La pseudo-console demande la position du curseur (`ESC[6n`) au démarrage et attend la réponse.
  Un harnais de test doit y répondre.
- Les touches peuvent arriver en `win32-input-mode` (`ESC[Vk;Sc;Uc;Kd;Cs;Rc_`) : voir `Key::parse` dans
  `crates/easytab-term/src/popup.rs`.
- Entrée dans la liste n'insère que si l'on s'est déplacé avec ↑/↓ ; sinon elle lance la
  commande. Tab insère toujours.
- mintty (fenêtre « Git Bash » par défaut) ne fournit pas de console : EasyTab s'y désactive.
  Utiliser Windows Terminal ou VS Code.
- Git Bash lancé par un programme Windows doit être un shell de connexion (`-l`) pour avoir
  `/usr/bin` dans le PATH.

## Fenêtre flottante (Windows)

- `easytab-term` lance `easytab-overlay.exe` et lui envoie la liste en JSON (protocole dans
  `crates/easytab-core/src/overlay.rs`). Si elle ne trouve pas le curseur, la liste est dessinée
  dans le terminal. `EASYTAB_OVERLAY=0` la désactive.
- Position du curseur (`caret.rs`) : d'abord la zone de saisie de VS Code (élément qui a le
  focus, de la taille d'une case), puis le curseur de texte UI Automation (Windows Terminal),
  puis le curseur système. Le curseur système de VS Code reste en début de ligne : ne pas le
  lire en premier.
- `placement::Tracker` ignore les petits écarts et les lectures en retard d'une lettre (sinon la
  fenêtre tremble). Le bord gauche du cadre est aligné sur le début du mot, comme Fig.
- Diagnostic : `EASYTAB_OVERLAY_LOG=fichier` journalise chaque lecture du curseur.
