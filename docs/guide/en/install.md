**English** · [Français](../fr/install.md)

# Install

The install script copies `easytab`, `easytab-term` and `easytab-overlay` to `~/.easytab/bin` and
adds EasyTab to the shell's config file, which also puts this folder in the PATH. Running the same
command again updates; so does `easytab update`, for the shells already set up. Each release
publishes a `SHA256SUMS`: the install scripts and `easytab update` check the archive before
unpacking it (a different archive stops the installation). Old versions set aside by an update
(`*.old-…` in `~/.easytab/bin`) are deleted the next time a terminal starts.

Supported shells: zsh, bash and PowerShell (7 and Windows PowerShell 5), and Git Bash on Windows,
in Windows Terminal or VS Code's terminal. In the default "Git Bash" window (mintty), which gives
Windows programs no console, EasyTab goes through `winpty`, shipped with Git for Windows; without
it, the shell starts there without EasyTab, with a message.

PowerShell: the profile loads `~/.easytab/bin/easytab-profile.ps1` (`. 'path'`), copied from the
archive by `easytab install --shell pwsh`; `easytab init pwsh` still prints it for profiles older
than this file. It contains no path, so that the published version can be signed.

## Company PCs

Antivirus software and AMSI distrust `irm … | iex` (download, then run), and an `AllSigned` or
`Restricted` execution policy refuses scripts. Without a script:

1. download `easytab-x86_64-pc-windows-msvc.zip` from the
   [latest release](https://github.com/AdamSellin/EasyTab/releases/latest) and extract it;
2. in the extracted folder: `.\easytab.exe install --shell pwsh` (and `--shell bash` for Git
   Bash), then open a new terminal.

With `AllSigned`, scripts must be signed, which releases are not yet (see
[Code signing](development.md#code-signing-windows)). If the antivirus still blocks a program,
report it as a false positive (Microsoft: <https://www.microsoft.com/wdsi/filesubmission>) or ask
the IT department to allow the publisher.

[← Contents](README.md)
