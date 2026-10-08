# EasyTab architecture

How EasyTab is built today (version 0.1.x). For installing, settings and the folder layout, see
[the guide](guide/en/README.md).

## Overview

EasyTab is a PTY wrapper, like Fig's `figterm`: it sits between the terminal and the shell, sees
every key and every byte the shell prints, works out the command line being typed, and shows
suggestions from the Fig specs. There is no daemon and no desktop app: each terminal tab runs its
own `easytab-term`, which may start its own `easytab-overlay`.

```
 terminal (Windows Terminal, VS Code, iTerm2, GNOME Terminal…)
   │  keys ▲ screen
   ▼       │
 easytab-term ── PTY / ConPTY ──► shell (zsh, bash, PowerShell, Git Bash)
   │   uses easytab-core            │ shell-integration/ prints OSC 133 / OSC 7
   │                                │ and sends aliases
   │ JSON lines (stdin / stdout)
   ▼
 easytab-overlay (optional floating window under the cursor)
```

The keyboard always stays with `easytab-term`. The floating window only displays the list and never
takes the focus.

## Crates

| Crate | Binary | Role |
|---|---|---|
| `easytab-core` | (library) | Shell state, completion engine, ranking, history, settings, overlay protocol. |
| `easytab-term` | `easytab-term` | PTY wrapper: starts the shell, relays keys and output, draws the list in the terminal or drives the overlay. |
| `easytab-overlay` | `easytab-overlay` | Floating window (tao + wry WebView, `popup.html`) placed under the terminal's cursor. |
| `easytab-cli` | `easytab` | `install`, `uninstall`, `update`, `config`, `doctor`, `init`. Does not depend on `easytab-core` (no QuickJS): it shares `config.rs` and `lang.rs` through `#[path]`. |

## Startup

1. `easytab install` adds a block to `~/.zshrc`, `~/.bashrc` or the PowerShell profile. The block is
   the matching file of `shell-integration/`, embedded in `easytab` with `include_str!`.
2. When an interactive shell starts outside EasyTab (`EASYTAB_TERM` unset), the block replaces it
   with `easytab-term --shell <shell>`. `EASYTAB_DISABLE=1` skips this.
3. `easytab-term` starts the same shell in a pseudo-terminal (`portable-pty`, ConPTY on Windows),
   with `EASYTAB_TERM` set. This time the block installs the prompt hooks: `OSC 133` marks (A
   prompt start, B input start, C command runs, D command done with exit code) and `OSC 7`
   (current folder).
4. Special cases: under mintty (Git Bash window), `easytab-term` runs under `winpty`; in a PowerShell
   profile, the integration calls `[Environment]::Exit` after `easytab-term` returns, so one `exit`
   closes the window.

## Reading the command line

`easytab-term` keeps a copy of the screen (`vt100`) in `Session` (`easytab-core/src/session.rs`).
The `OSC 133` marks tell where the prompt ends; the text between mark B and the cursor is the
current line. `line.rs` splits it into words like a POSIX shell (simplified). Full-screen programs
(alternate screen) and running commands (between C and D) get no suggestions.

## From a line to suggestions

`Completer::complete_words` (`complete.rs`) walks the spec of the command along the words already
typed, then lists what can replace the current word. Sources, in order:

1. User specs in `~/.easytab/specs` (`spec::custom`).
2. On Windows, hand-written specs for Windows tools (`specs/windows.json`).
3. The hand-written spec of the `easytab` command itself (`specs/easytab.json`), checked against
   its clap definition by a test of `easytab-cli`.
4. Fig specs, imported by `tools/import-fig-specs.mjs` and embedded compressed
   (`specs/specs.json.z`, `loadable.json.z`, `modules.json.z`).
5. PowerShell commands described by PowerShell itself (`pwsh.rs`, cached in
   `~/.easytab/cache/powershell.json`).
6. fish (`complete -C`) and bash-completion (`shell.rs`).
7. The command's `--help` output (`help.rs`, cached in `~/.easytab/cache/help.json`).

Other suggestion kinds: files and folders (`files.rs`), values read from project files where the Fig
generators do nothing or need `bash`/`cat` (`project.rs`: Makefile, package.json, composer.json,
angular.json, `~/.ssh/config`), whole commands from the history (`history.rs`), saved workflows
with fields (`workflow.rs`), and the corrected command when one fails on a typo.

Ranking (`rank.rs`): match quality (prefix, then fuzzy) and how often each suggestion was picked
(`~/.easytab/usage.json`).

## Dynamic suggestions

Fig generators (git branches, npm scripts, docker containers…) run a command and pass its output to
the spec's JavaScript. `generators.rs` does this on a separate thread with an embedded QuickJS
(`rquickjs`), so typing never waits. `exec.rs` runs the commands with a timeout and never runs the
user's own line. Results are cached: the list shows what is already known and is refreshed when new
results arrive.

## Threads in `easytab-term`

| Thread | Job |
|---|---|
| main | starts everything, waits for the shell to exit |
| keyboard → shell | reads keys (`Key::parse`, including Windows `win32-input-mode`), handles them while the list is open, forwards the rest |
| shell → screen | forwards output, updates the screen copy, redraws the list |
| resize | follows the terminal size (also on Windows, which has no `SIGWINCH`) |
| generators | redraws when dynamic suggestions arrive |
| overlay fallback | switches to the in-terminal list when the overlay reports `unavailable` |

Shared state sits behind one `Mutex<Shared>`.

## Showing the list

- **In the terminal** (`easytab-term/src/popup.rs`): drawn with ANSI sequences under (or above) the
  line. Covered cells are restored from the screen copy. Works everywhere: SSH, Wayland, tmux.
- **Floating window** (`easytab-overlay`): `easytab-term` starts it and sends `show`/`hide` requests
  as JSON lines; the window answers `ready` or `unavailable` (`easytab-core/src/overlay.rs`). If the
  program is missing, the cursor cannot be found or `EASYTAB_OVERLAY=0`, the list falls back to
  the terminal.

Finding the cursor on screen (`easytab-overlay/src/platform/`):

| System | Cursor position |
|---|---|
| Windows | VS Code's input element, then UI Automation (Windows Terminal), then the system caret (`win/caret.rs`) |
| macOS | Accessibility (`AXBoundsForRange`) when allowed, otherwise estimated from the window frame and the grid size (`estimate.rs`); points, not pixels |
| Linux | X11 only (`_NET_ACTIVE_WINDOW`), estimated like macOS. Not started under Wayland |

`placement::Tracker` ignores small moves and readings one letter late so the window does not
shake; the frame's left edge lines up with the start of the word.

## Files on disk

All under `~/.easytab/`:

| Path | Content |
|---|---|
| `bin/` | `easytab`, `easytab-term`, `easytab-overlay`, `easytab-profile.ps1` |
| `config.toml` | settings (`config.rs`) |
| `workflows.toml` | saved workflows |
| `specs/` | user specs |
| `history.jsonl` | commands run under EasyTab, with folder, exit code and duration |
| `usage.json` | how often each suggestion was picked |
| `cache/` | `--help` and PowerShell answers |

Nothing leaves the machine, except `easytab update`, which downloads the latest release from GitHub
and checks it against the release's `SHA256SUMS`.

## Choices

- **Two renderers, one engine.** The in-terminal list works everywhere; the floating window looks
  like Fig where the system lets us find the cursor. Same engine and keyboard handling for both.
- **No daemon.** Each `easytab-term` loads the embedded specs itself. Nothing to start, keep alive
  or version separately.
- **tao + wry instead of Tauri.** The overlay is one transparent window with one page; it does not
  need Tauri's bundler, updater or settings window.
- **Plain files instead of SQLite.** JSON lines and small JSON files are enough for the history and
  the usage counts, and keep the binaries small.
- **Fig specs reused as is** (MIT, notice in `specs/LICENSE-fig`), with QuickJS for their
  generators.

## Tests

- Unit tests sit next to the code (`mod tests`).
- `easytab-term/tests/e2e.rs` runs `easytab-term` in a real pseudo-terminal and types keys: bash on
  Linux, PowerShell and Git Bash on Windows.
- CI (`.github/workflows/ci.yml`) runs `cargo fmt`, `clippy -D warnings`, the tests and a release
  build on Linux, macOS and Windows.
