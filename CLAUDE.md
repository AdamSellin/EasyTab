# EasyTab : notes pour Claude

Autocomplétion graphique pour le terminal, façon Fig : un wrapper PTY lit la ligne en cours et
affiche des suggestions tirées des specs de Fig. Vue d'ensemble des dossiers : `docs/guide.md`.

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
- La fenêtre flottante (`crates/easytab-overlay`) demande sous Linux WebKitGTK
  (`apt install libwebkit2gtk-4.1-dev libgtk-3-dev`). Vérifier les autres systèmes depuis Linux :
  `cargo clippy -p easytab-overlay --target x86_64-pc-windows-gnu` et, après
  `rustup target add aarch64-apple-darwin`,
  `CC_aarch64_apple_darwin=clang cargo clippy -p easytab-overlay --target aarch64-apple-darwin`.
  `easytab-term` ne se compile pas ainsi (QuickJS demande un gcc MinGW).
- Publier une version : « Run workflow » sur `Release` dans l'onglet Actions, avec le nom de la
  version (`v0.1.1`), ou pousser un tag `v*`. Passer d'abord `version` dans `Cargo.toml` au même
  numéro : la release le corrige d'elle-même, mais une compilation locale l'affiche.

- Réglages de l'utilisateur : `crates/easytab-core/src/config.rs` (`~/.easytab/config.toml`),
  partagé avec `easytab-cli` par `#[path]` pour ne pas y embarquer QuickJS. Tout nouveau réglage
  va aussi dans `TEMPLATE` (anglais) et `TEMPLATE_FR`, et dans `docs/guide.md` (le README reste
  court). `README.md` est en anglais, `README.fr.md` en français : changer les deux ensemble.
- Ordre des specs d'une commande (`Completer::complete_words`) : `~/.easytab/specs` (`spec::custom`),
  puis, sous Windows, `specs/windows.json` (`spec::windows`, écrit à la main), puis Fig, puis
  PowerShell (`pwsh.rs`), puis fish et bash-completion (`shell.rs`), puis `--help` (`help.rs`).
  Dans PowerShell, un alias (`ls`) passe avant les specs embarquées : il reçoit les paramètres de
  sa cmdlet. Valeurs lues dans les fichiers du projet ou de l'utilisateur (Makefile,
  composer.json, angular.json, package.json, `~/.ssh/config` et `known_hosts`) : `project.rs`,
  seulement là où les generators Fig ne font rien ou demandent `bash`/`cat` ; leurs doublons sont
  écartés.

## Conventions

- Code, commentaires, commits et PR en français. Les messages affichés à l'utilisateur sont
  bilingues : anglais par défaut, français si le système l'est (`EASYTAB_LANG` force le choix).
  En Rust, `tr("english", "français")` ou `tr!("… {x}", "… {x}")` de
  `crates/easytab-core/src/lang.rs` (partagé avec `easytab-cli` par `#[path]`) ; dans les
  scripts d'installation, `say`/`Tr`.
- Une PR par sujet, fusionnée en squash. Tests unitaires dans le fichier du code (`mod tests`).
- `shell-integration/easytab.ps1` et les chaînes de `scripts/install.ps1` restent en ASCII :
  Windows PowerShell 5.1 lit l'UTF-8 sans BOM comme de l'ANSI, et `irm | iex` refuse un BOM.

## Windows, les pièges connus

- Toolchain Rust MSVC (`rustup default stable-msvc`) ; la GNU échoue sur `dlltool`.
- La pseudo-console demande la position du curseur (`ESC[6n`) au démarrage et attend la réponse.
  Un harnais de test doit y répondre.
- Les touches peuvent arriver en `win32-input-mode` (`ESC[Vk;Sc;Uc;Kd;Cs;Rc_`) : voir `Key::parse` dans
  `crates/easytab-term/src/popup.rs`.
- Entrée dans la liste insère la suggestion surlignée si elle complète le mot tapé (`git sta`
  → `status`), si l'on s'est déplacé avec ↑/↓, ou si c'est une sous-commande juste après une
  insertion depuis la liste (`dock` Entrée → `docker-compose `, Entrée → `up`) ; sinon elle
  lance la commande. Tab insère toujours.
- mintty (fenêtre « Git Bash » par défaut) ne fournit pas de console : l'intégration bash y lance
  `easytab-term` sous `winpty` (détecté par `TERM_PROGRAM=mintty`). Sans winpty, EasyTab s'y
  désactive. Le test de bout en bout ne couvre pas ce cas : tester à la main.
- Git Bash lancé par un programme Windows doit être un shell de connexion (`-l`) pour avoir
  `/usr/bin` dans le PATH.

## Fenêtre flottante

- `easytab-term` lance `easytab-overlay` et lui envoie la liste en JSON (protocole dans
  `crates/easytab-core/src/overlay.rs`). Si elle ne trouve pas le curseur, la liste est dessinée
  dans le terminal. `EASYTAB_OVERLAY=0` la désactive.
- Code commun dans `app.rs` (fenêtre tao, page wry), `placement.rs`, `estimate.rs` ; par système
  dans `platform/` (`win`, `macos`, `linux`) : curseur, écran, fenêtre au premier plan.
- macOS : curseur par l'accessibilité (`AXBoundsForRange`) si l'autorisation est donnée, sinon
  déduit du cadre de la fenêtre et de la taille du terminal (`term_cols`, `term_rows`).
  Coordonnées en points. Linux : X11 seulement (`_NET_ACTIVE_WINDOW`), position déduite de la
  même façon ; pas lancée d'office sous Wayland.
- `placement::Tracker` ignore les petits écarts et les lectures en retard d'une lettre (sinon la
  fenêtre tremble). Le bord gauche du cadre est aligné sur le début du mot, comme Fig.
- Diagnostic : `EASYTAB_OVERLAY_LOG=fichier` journalise chaque lecture du curseur.
- Windows, position du curseur (`platform/win/caret.rs`) : d'abord la zone de saisie de VS Code
  (élément qui a le focus, de la taille d'une case), puis le curseur de texte UI Automation
  (Windows Terminal), puis le curseur système. Le curseur système de VS Code reste en début de
  ligne : ne pas le lire en premier.
