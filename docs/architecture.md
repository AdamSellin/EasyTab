# EasyTab : architecture d'une autocomplétion graphique pour le terminal (type Fig)

*Rédigé le 2026-10-04. État du repo `AdamSellin/EasyTab` : vide, aucun commit.*

## 1. En bref

- **Le cœur n'est pas la fenêtre graphique, c'est un « wrapper » de pseudo-terminal (PTY)** qui s'intercale entre le terminal et le shell. Il voit chaque frappe et chaque octet affiché, sait ce que l'utilisateur est en train de taper et peut intercepter Tab / ↑ / ↓ / Entrée quand la liste de suggestions est ouverte. C'est exactement ce que faisait Fig (`figterm`).
- **Deux façons d'afficher la liste** :
  - **A. Overlay GUI** : une petite fenêtre transparente, toujours au premier plan, positionnée au pixel près sous le curseur (le vrai « style IDE » de Fig). Très joli, mais dépend des API d'accessibilité de chaque OS et ne marche pas partout (Wayland, SSH, certains terminaux).
  - **B. Popup inline (TUI)** : le wrapper dessine la liste directement dans le terminal avec des séquences ANSI (approche de Microsoft *inshellisense*). Marche partout, sans permissions, y compris en SSH et sous Wayland.
- **Recommandation : un moteur commun en Rust + les deux rendus.** On commence par B (MVP multiplateforme rapide et robuste), puis on ajoute A sur macOS et Windows, avec B comme repli automatique quand l'overlay est impossible.
- **Ne pas réécrire les specs de complétion** : réutiliser `withfig/autocomplete` (licence MIT, plusieurs centaines de CLI déjà décrites : git, docker, npm, kubectl…).

## 2. Comment fonctionnait Fig (et ce qu'on en garde)

| Brique Fig | Rôle | Équivalent EasyTab |
|---|---|---|
| Intégration shell (scripts sourcés dans `.zshrc`, `.bashrc`, `config.fish`) | Lance le wrapper, émet des marqueurs de prompt | `shell-integration/` |
| `figterm` (wrapper PTY en Rust) | Lit l'écran, extrait la ligne de commande en cours, intercepte les touches | crate `easytab-term` |
| Daemon / app desktop | Moteur de complétion, réglages, mises à jour | crate `easytab-core` + app Tauri |
| Fenêtre WebView transparente | Affiche les suggestions près du curseur | overlay Tauri (approche A) |
| API d'accessibilité macOS | Trouve la position du curseur en pixels | AX (macOS), UI Automation (Windows), X11 (Linux) |
| Specs `withfig/autocomplete` | Description déclarative de chaque CLI | réutilisées telles quelles |

Fig a été racheté par AWS et intégré à Amazon Q Developer CLI, dont le code Rust a été publié en open source : c'est une excellente référence à lire (à vérifier : licence exacte et dépôt qui contient la partie desktop/autocomplétion). Autres références utiles : **inshellisense** (Microsoft, MIT, approche B, utilise les specs Fig), **carapace** (moteur de complétion multi‑shell en Go), **ble.sh** et **zsh-autocomplete** (complétion « live » purement dans le shell).

## 3. Architecture recommandée

```
 ┌──────────────── Terminal (Windows Terminal, iTerm2, GNOME Terminal…) ───────────────┐
 │                                                                                      │
 │   easytab-term (wrapper PTY)  ──── PTY / ConPTY ────►  shell (zsh, bash, fish, pwsh)  │
 │     │  • parse la sortie (vte) → grille écran + position curseur                     │
 │     │  • marqueurs OSC 133 → début/fin du prompt → ligne de commande en cours         │
 │     │  • intercepte Tab/↑/↓/Entrée/Échap quand la popup est ouverte                   │
 │     │  • rendu B : popup inline en ANSI                                               │
 └─────┼────────────────────────────────────────────────────────────────────────────────┘
       │ IPC (socket Unix / named pipe Windows, messages protobuf ou JSON)
       ▼
  easytab-daemon (Rust)
   • moteur de complétion : tokenizer shell → parcours de la spec → candidats
   • runtime JS embarqué (rquickjs ou deno_core) pour les specs Fig et leurs generators
   • classement : fuzzy (crate nucleo) + fréquence d'usage (historique SQLite local)
   • cache des generators (ex. branches git), timeouts
       │
       ▼
  easytab-app (Tauri v2)
   • overlay transparent sans bordure, always-on-top, qui ne prend JAMAIS le focus
   • position = géométrie fenêtre terminal (API OS) + (ligne, colonne) curseur × taille de cellule
   • fenêtre de réglages, onboarding (permissions), icône de barre d'état, mises à jour
```

Point clé : **même en mode overlay, ce sont toujours le wrapper PTY qui capte le clavier**. La fenêtre graphique ne fait qu'afficher ; elle ne doit jamais voler le focus au terminal.

### 3.1 Savoir ce que l'utilisateur tape

Deux techniques, à combiner :

1. **Lecture d'écran + marqueurs de prompt (méthode Fig, recommandée par défaut)** : l'intégration shell émet `OSC 133;A` (début du prompt) et `OSC 133;B` (fin du prompt / début de saisie). Le wrapper garde une copie de l'écran (crate `vte` ou `vt100`) ; le texte entre le marqueur B et le curseur est la ligne en cours. Fonctionne pour tous les shells sans dépendre de leurs API internes.
2. **Hooks natifs du shell (plus fiable quand dispo)** : `zle` widgets en zsh (`$BUFFER`, `$CURSOR`), `commandline` en fish, PSReadLine en PowerShell. En bash c'est difficile (pas de hook par frappe) : on reste sur la lecture d'écran.

Pièges : prompts multi-lignes, `RPROMPT` à droite, caractères larges (emoji, CJK), lignes qui débordent, applications plein écran (vim, htop : détecter l'*alternate screen* et désactiver), tmux (fonctionne si le wrapper est lancé dans chaque pane).

### 3.2 Les specs de complétion

- Une spec Fig est un objet TypeScript : `name`, `subcommands`, `options` (`-v`, `--verbose`, avec `args`), `args` avec `template` (`filepaths`, `folders`) ou `generators` (une commande shell + une fonction `postProcess` en JS qui transforme sa sortie en suggestions).
- Le paquet compilé (`@withfig/autocomplete` sur npm) contient des modules JS. Il faut donc un **runtime JS embarqué** : `rquickjs` (léger, ~1 Mo) pour commencer, `deno_core` si on a besoin de plus de compatibilité.
- **Repli pour les commandes sans spec** : demander au shell lui-même (`fish` sait le faire avec `complete -C "git ch"`, zsh via capture de `compsys`, carapace en bridge), ou parser `--help`.
- **Sécurité** : les generators exécutent des commandes. Les limiter aux specs fournies (code de confiance), avec timeout, sans jamais exécuter la ligne de l'utilisateur.

### 3.3 Positionner l'overlay (approche A) selon l'OS

| OS | Position de la fenêtre du terminal | Position du curseur | Difficultés |
|---|---|---|---|
| **macOS** | API Accessibility (`AXUIElement`) | AX quand le terminal l'expose (Terminal.app, iTerm2), sinon calcul grille × taille de cellule | Permission « Accessibilité » à faire accorder par l'utilisateur ; signature + notarisation Apple (compte développeur payant) |
| **Windows** | Win32 (`GetWindowRect`) / UI Automation | UIA `TextPattern` (Windows Terminal le supporte), sinon calcul | ConPTY obligatoire ; `cmd.exe` n'a aucun hook ; certificat de signature sinon alerte SmartScreen |
| **Linux X11** | Géométrie fenêtre via X11 | Calcul grille × cellule | Faisable, mais chaque terminal a ses marges/paddings |
| **Linux Wayland** | Interdit par conception (une app ne peut ni lire la position d'une autre fenêtre ni placer la sienne librement) | — | Pistes : moteur IBus (méthode de saisie, qui reçoit la position du curseur ; Fig l'utilisait sur Linux), extensions par compositeur. **Sinon repli sur l'approche B.** |

Calcul générique quand l'OS ne donne pas le curseur : `x = fenêtre.x + padding + colonne × largeur_cellule`, `y = fenêtre.y + barre_titre + padding + (ligne + 1) × hauteur_cellule`. Le wrapper connaît (ligne, colonne) ; `TIOCGWINSZ` donne parfois la taille en pixels (`ws_xpixel`/`ws_ypixel`), sinon taille fenêtre ÷ nombre de colonnes.

### 3.4 Stack conseillée

| Besoin | Choix | Pourquoi |
|---|---|---|
| Langage principal | **Rust** | Latence < 10 ms par frappe, binaire unique, multiplateforme, même choix que Fig/Q |
| PTY multiplateforme | `portable-pty` (projet WezTerm) | Gère PTY Unix et ConPTY Windows |
| Parsing terminal | `vte` ou `vt100` | Reconstruit la grille écran |
| Rendu inline (B) | `crossterm` | ANSI portable, Windows inclus |
| Overlay + réglages (A) | **Tauri v2** | WebView natif léger (pas d'Electron), fenêtres transparentes, bundler MSI/DMG/deb/AppImage, updater intégré |
| UI de la popup | HTML/CSS (Svelte ou Solid) | Icônes, descriptions, style VS Code |
| JS pour les specs | `rquickjs` | Embarqué, léger |
| Fuzzy matching | `nucleo` | Le moteur de Helix, très rapide |
| Historique / stats | SQLite (`rusqlite`) | Local, rien ne sort de la machine |
| API OS | crates `objc2`/`accessibility-sys` (macOS), `windows` (UIA), `x11rb` (Linux) | Accès natif |

### 3.5 Installation et distribution

- **macOS** : `.dmg` signé + notarisé, Homebrew cask. Onboarding qui guide vers la permission Accessibilité.
- **Windows** : `.msi` signé (bundler Tauri), puis winget.
- **Linux** : `.deb`, `.rpm`, AppImage. Éviter Flatpak/Snap : le sandbox gêne le wrapper PTY et l'accès aux shells.
- **Installation shell** : la commande `easytab install` ajoute une ligne à `.zshrc` / `.bashrc` / `config.fish` / profil PowerShell, réversible avec `easytab uninstall`. Le wrapper ne s'active que dans les sessions interactives.
- Mises à jour : updater Tauri ; specs mises à jour séparément (paquet de specs versionné).

## 4. Plan par étapes

1. **Squelette** : workspace Cargo (`easytab-core`, `easytab-term`, `easytab-daemon`, `easytab-app`), dossier `shell-integration/`, CI GitHub Actions qui build sur les 3 OS.
2. **Wrapper PTY passthrough** (Linux/macOS, zsh + bash) : le terminal fonctionne exactement comme sans EasyTab, mesure de latence.
3. **Extraction de la ligne** : marqueurs OSC 133 + lecture d'écran, tests sur prompts multi-lignes, Starship, Oh My Zsh.
4. **Moteur de specs** : tokenizer shell, chargement des specs Fig dans rquickjs, suggestions statiques (sous-commandes, options), templates fichiers/dossiers.
5. **Popup inline (B)** : affichage sous la ligne, navigation clavier, insertion de la suggestion, Échap. → **premier MVP utilisable.**
6. **Generators et classement** : exécution avec timeout et cache, fuzzy + fréquence d'usage.
7. **fish, PowerShell et Windows (ConPTY)**, Git Bash, WSL.
8. **Overlay GUI (A)** : Tauri sur macOS (AX), puis Windows Terminal (UIA), puis Linux X11 ; repli automatique sur B ailleurs.
9. **App desktop** : réglages (thème, raccourcis, shells activés), onboarding permissions, icône barre d'état.
10. **Distribution** : signature, notarisation, installeurs, updater, `easytab doctor` pour diagnostiquer une intégration cassée.
11. **Durcissement** : tmux, SSH (le wrapper ne tourne pas sur la machine distante : désactiver proprement ou proposer d'installer EasyTab côté serveur), apps plein écran, performances.

## 5. Risques principaux

- **Latence** : chaque frappe passe par le wrapper ; tout ce qui dépasse quelques ms se sent. Le rendu et les generators doivent être asynchrones.
- **Mauvaise détection de la ligne** : c'est la source n°1 de bugs (prompts exotiques). Investir tôt dans des tests sur captures d'écran terminal.
- **Wayland** : pas d'overlay fiable aujourd'hui ; d'où l'intérêt du mode inline dès le départ.
- **Permissions macOS et signature** : coût (compte Apple, certificat Windows) et friction d'onboarding.
- **Licences** : specs Fig en MIT (garder la notice) ; vérifier la licence de tout code repris d'Amazon Q.
