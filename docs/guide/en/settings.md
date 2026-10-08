**English** · [Français](../fr/settings.md)

# Settings

`easytab config` creates `~/.easytab/config.toml`, with every setting commented; `easytab doctor`
reports an error in this file. Reopen the terminals after a change.

```toml
[list]
rows = 8            # visible suggestions (3 to 20)
overlay = true      # floating window, or false for the list in the terminal
theme = "dark"      # or "light"
icons = "badges"    # or "emoji" (list in the terminal)
history = true      # suggest commands typed before
inline = true       # grey suggestion after the cursor, accepted with →
shell = true        # commands without a spec: bash-completion or fish completions
help = true         # commands without a spec: options read from "command --help"
correct = true      # after a typo, the fixed command in grey at the next prompt
next = true         # after git commit, git push in grey at the next prompt (and git tag…)

[keys]
enter_inserts = true  # false: Enter always runs the command, only Tab inserts
search = true         # Ctrl+R searches the history; false: Ctrl+R stays with the shell
```

The `EASYTAB_OVERLAY` and `EASYTAB_ICONS` variables take precedence over the file.

Message language (`easytab` command, install scripts, `config.toml` template): English by
default, French if the system is (`LC_ALL`, `LC_MESSAGES` or `LANG` starting with `fr`; without
these variables, the language of Windows or macOS). `EASYTAB_LANG=fr` or `EASYTAB_LANG=en` forces
the choice.

Turning it off: `EASYTAB_DISABLE=1` for one session, `easytab uninstall` for good.

[← Contents](README.md)
