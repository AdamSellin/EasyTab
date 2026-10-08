**English** · [Français](../fr/workflows.md)

# Workflows

Saved commands with fields to fill in, written between braces, in `~/.easytab/workflows.toml`
(`easytab workflows` creates it with examples):

```toml
[[workflow]]
name = "Shell in a container"
command = "docker exec -it {container} bash"

[[workflow]]
name = "Branch from main"
command = "git switch -c {branch} main"
description = "New branch, started from main"   # optional
```

A workflow is suggested in the list (▸ icon) as soon as the start of its command is typed
(`docker ex`), and in the Ctrl+R search by its name or its command. Picking it writes the command
up to the first field; the rest shows in grey (`{container} bash`). Type the value of the field,
with the help of the usual list (here, docker's containers), then → adds the rest up to the next
field. `${HOME}` and `{a,b}` stay plain text for the shell. The file is read when the terminal
opens; `easytab doctor` reports an error in it.

[← Contents](README.md)
