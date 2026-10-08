**English** · [Français](../fr/suggestions.md)

# Where suggestions come from

**Fig specs.** Over 700 commands (git, docker, npm, symfony, gradle…) are described by the specs
of [withfig/autocomplete](https://github.com/withfig/autocomplete), embedded in EasyTab (see
[specs/README.md](../../../specs/README.md)).

**Windows tools.** On Windows, `specs/windows.json` describes the tools missing from the Fig
specs: winget, wsl, choco, scoop, ipconfig, netsh, robocopy, taskkill, tasklist, sc, dism, sfc,
where, findstr, xcopy, icacls, schtasks, shutdown, systeminfo, nslookup, ping, tracert and net.
They come before Fig's (whose `ping` and `where` describe the Unix versions). Options starting
with `/` are completed ignoring case (`/mir` is `/MIR`), with their value attached
(`/LOG:log.txt`, `/FeatureName:…`).

**History.** Commands typed before that extend the current line come first
(`docker-compose u` → `docker-compose up -d --build`, clock icon), taken from the shell's history
(`~/.bash_history`, `~/.zsh_history`, PowerShell's PSReadLine history). EasyTab also keeps, in
`~/.easytab/history.jsonl`, the folder, exit code and duration of each command run under it: those
run in the current folder come first, those whose last run failed come last. `history = false`
turns both off.

**Environment variables.** `$HO` suggests `$HOME`, `$HOSTNAME`… with their value; `${HO` gives
`${HOME}`. In PowerShell, it is `$env:PA` → `$env:PATH`. These are the variables known when the
terminal opens: a variable exported later in the session is not there.

**Aliases.** In bash and zsh, the integration sends the shell's aliases to EasyTab at the first
prompt, then when they change (an `OSC 6973` sequence, ignored by terminals). With `alias g=git`,
`g pu` is completed like `git pu`, and aliases are suggested as commands, with their value as the
description. PowerShell's aliases come from `Get-Command`.

**Ranking.** Suggestions are ranked by match quality (start of the name, then fuzzy search:
`git chk` finds `checkout`), then by how often they are used, kept in `~/.easytab/usage.json`.
Fuzzy search is only used if no suggestion starts with or contains the typed word: `git che`
suggests `checkout`, not `credential-helper-selector`.

**Computed values.** Branches, scripts, containers… come from the *generators* of the Fig specs:
EasyTab runs the command the spec provides (`git branch`, reading `package.json`…) in the
background, passes its output to the spec's JavaScript in an embedded JS engine (QuickJS), then
completes the list as soon as the result arrives. Typing never waits. Some specs compute part of
their subcommands this way (Fig's `generateSpec`: the commands of `composer`, those of
`php bin/console` in a Symfony project…); the result is kept for one minute per folder.

**Project files.** A few values are read straight from files, without running a command (so also
on Windows without `bash`):

- the `Makefile` targets for `make` (with their `## …` comment as the description);
- the `composer.json` scripts for `composer run-script` / `composer run`;
- the `angular.json` projects for `ng build`, `ng serve`, `ng test`…;
- the `package.json` scripts (also looked up in parent folders) for `npm run`, `yarn` /
  `yarn run`, `pnpm` / `pnpm run` and `bun run`;
- SSH hosts for `ssh`, `sftp` and `scp` (`host:`): `Host` lines of `~/.ssh/config` (and of the
  files it `Include`s, except `*` patterns) and hosts of `~/.ssh/known_hosts` (except hashed
  entries). After `user@`, only the host is completed.

Files are read again when they change. `docker compose` services come from the Fig specs.

**PowerShell.** PowerShell commands (`Get-ChildItem`…) and their parameters come from PowerShell
itself (`Get-Command`), started once in the background. Aliases (`ls`, `gci`, `cat`…) are suggested
with the command they stand for and get its parameters (`ls -Recurse`): in PowerShell, `ls` is
`Get-ChildItem`, not Unix's `ls`. An alias to a program (`g` → `git`) gets that program's spec.

**Shell completions.** For a command without a spec, EasyTab asks fish, then bash-completion, if
they are installed, what they would suggest after Tab: `fish -c 'complete -C …'` (with the
descriptions), then the completion function bash-completion declares for the command (including
the user's, in `~/.local/share/bash-completion/completions`). They run in the background, without
the user's configuration, for 2 seconds at most; the answer is kept per folder and per line. A
command neither of them knows goes on to `--help`. On Windows, only in Git Bash; never in
PowerShell, which describes its commands itself. `shell = false` in the settings turns it off.

**`--help`.** For an installed command with no spec and no shell completion, EasyTab runs
`command --help` once in the background (3 seconds at most, from the temporary folder) and reads
its options from it (`-x, --option=VALUE  description`). The answer is kept in
`~/.easytab/cache/help.json` as long as the program does not change. Only commands in the PATH are
asked, never `rm`, `dd`, `shutdown`, `reboot`, `halt`, `poweroff`, `mkfs`, `format`, nor `.bat` /
`.cmd` scripts on Windows; output that does not look like help (exit code other than 0 or 1, fewer
than two options) is ignored. `help = false` in the settings turns it off.

## Custom specs

The `~/.easytab/specs/*.json` files describe commands in addition to Fig's, or instead of them: a
spec with the same name replaces the embedded one. A command described this way is suggested even
if it is not in the PATH (a shell function or alias). The files are read when the terminal opens;
an error is written to the log (`EASYTAB_LOG`).

The format is the one of the embedded specs (converted Fig specs): `names` (always an array),
`description`, `subcommands`, `options` and `args`. An argument can have `suggestions`
(`{"names": [...], "description": ...}`), `templates` (`"filepaths"`, `"folders"`), and be
`optional`, `variadic` or `is_command`; an option can be `persistent` (valid in subcommands) or
`requires_equals` (`--opt=value`); `"load": "git"` reuses another spec. A file holds one spec, or
an array of specs.

```json
{
  "names": ["deploy"],
  "description": "Deploys the application",
  "subcommands": [
    {
      "names": ["app", "a"],
      "description": "Deploys the web application",
      "args": [{ "name": "env", "suggestions": [{ "names": ["staging"] }, { "names": ["prod"] }] }]
    }
  ],
  "options": [
    { "names": ["-f", "--force"], "description": "No confirmation" },
    { "names": ["--config"], "args": [{ "name": "file", "templates": ["filepaths"] }] }
  ]
}
```

[← Contents](README.md)
