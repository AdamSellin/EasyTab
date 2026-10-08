**English** · [Français](../fr/list.md)

# The list

| Key | Action |
|---|---|
| ↑ / ↓, Shift+Tab | Pick a suggestion (Shift+Tab goes up) |
| Tab | Insert the picked suggestion |
| Enter | Insert the highlighted suggestion when it completes the typed word, after ↑ / ↓, when it is a subcommand and nothing is typed in the word yet (`go ` Enter → `build`), or right after picking a command or a subcommand from the list (`dock` Enter → `docker-compose `, Enter → `up`); otherwise the command runs as usual |
| Esc | Close the list until the next key |
| Ctrl+Space | Reopen the list closed with Esc |
| → | Accept the grey suggestion |
| Ctrl+R | Search the whole history (Ctrl+R again: back to the usual list) |

The grey suggestion, after the cursor, is the most recent history command that extends the line.
It is drawn in the terminal, even with the floating window, and only when the cursor is at the end
of the line; → accepts it, otherwise → moves the cursor as usual. If the shell already shows its
own suggestion (PowerShell predictions), EasyTab does not add its own.

**Search (Ctrl+R).** The typed line becomes a search through the whole history: the commands that
contain each of its words, in any order and ignoring case, most recent first (`dock up` finds
`docker compose up -d`). The description gives the folder the command ran in, whether it failed,
how long it took and when. Tab or Enter replaces the line with the picked command, without running
it; Esc or Ctrl+R leaves the search. Matching workflows come first. `search = false` in `[keys]`
leaves Ctrl+R to the shell.

**Typo fixes.** When a command fails because of a typo, the fixed command shows in grey at the next
prompt, and → accepts it: `gti status` → `git status` (command not found, exit code 127 in bash
and zsh, any failure in PowerShell), `git stauts` → `git status` (unknown subcommand of a spec). A
typo is one letter too many, missing or changed, or two swapped letters (at most two typos in a
word longer than four letters); on a tie, the most used name wins. Nothing is suggested for a line
with `|`, `;`, `&` or `$`. `correct = false` in `[list]` turns it off.

**Next command.** After a successful command, the usual follow-up shows in grey at the next prompt,
and → accepts it: `git push` after `git commit`, `git push origin v1.2` after `git tag v1.2` (or
`git tag -a v1.2 …`), `git push -u origin name` after `git switch -c name` or
`git checkout -b name`. `next = false` in `[list]` turns it off.

When the list is closed, Tab, Shift+Tab and Ctrl+Space keep their usual behaviour (shell
completion), except Ctrl+Space right after Esc.

The list is shown in a floating window (`easytab-overlay`), placed under the terminal's cursor:
rounded corners, a shadow, icons by kind (git branch, npm script, folder, file, option…) and the
description at the bottom. The window never takes the focus: the keyboard stays with the terminal.
If it cannot find the cursor (or with `EASYTAB_OVERLAY=0`), the list is drawn in the terminal. To
understand a wrong placement, `EASYTAB_OVERLAY_LOG=file` logs every cursor position the window
reads.

- Windows: Windows Terminal and VS Code, cursor read through UI Automation.
- macOS: cursor read through accessibility if the terminal has access to it (System Settings >
  Privacy & Security > Accessibility: Terminal, iTerm…, which `easytab-overlay` inherits);
  otherwise, the position is worked out from the terminal window and its size in columns and rows.
- Linux: X11 sessions only, position worked out from the active window. Under Wayland, the list
  stays in the terminal (`EASYTAB_OVERLAY=1` tries anyway, for an XWayland terminal).

In the terminal, the list shows a coloured badge by kind (`>` command, `$` subcommand, `-` option,
`@` computed value, `/` folder…), the typed letters in bold, the expected arguments in grey
(`--cleanup <mode>`) and the description of the picked suggestion at the bottom. With
`EASYTAB_ICONS=emoji`, the badges become emoji (📦 🚩 🌿 📁…).

[← Contents](README.md)
