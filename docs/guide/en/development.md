**English** · [Français](../fr/development.md)

# Development

## Building from source

Requirements: stable [Rust](https://rustup.rs). On Windows, the MSVC toolchain
(`rustup default stable-msvc`). On Linux, the floating window needs WebKitGTK:
`sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev` (Debian, Ubuntu).

```sh
cargo build --release
./target/release/easytab install      # the current shell (PowerShell on Windows, except Git Bash)
# open a new terminal, then:
easytab doctor
```

After a new build, run `./target/release/easytab install` again to update the installed copy.
Terminals already open keep the old version until they are closed.

To see what EasyTab detects while you type:

```sh
EASYTAB_LOG=/tmp/easytab.log zsh      # then, in another terminal:
tail -f /tmp/easytab.log
```

Before each push, like the CI (Linux, macOS and Windows):

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check advisories licenses sources   # cargo install cargo-deny
```

Rust 1.88 at least (`rust-version` in `Cargo.toml`, checked by the CI).

## Code layout

| Folder | Role |
|---|---|
| `crates/easytab-core` | Follows the shell's state (prompt markers `OSC 133`, current folder `OSC 7`, a copy of the screen) and turns the line being typed into suggestions from the specs. |
| `crates/easytab-term` | PTY wrapper: runs the shell in a pseudo-terminal, relays keyboard and screen, draws the suggestion list. |
| `crates/easytab-overlay` | Floating window, placed under the terminal's cursor. |
| `crates/easytab-cli` | The `easytab` command: `install`, `uninstall`, `update`, `config`, `doctor`, `init`. |
| `shell-integration/` | zsh, bash and PowerShell scripts that restart the shell under `easytab-term` and emit the prompt markers. |
| `specs/` | Completion specs imported from Fig, and those of the Windows tools (`windows.json`). |
| `tools/` | Script that imports the Fig specs. |

The architecture and its design choices are described in [architecture.md](../../architecture.md).

## Releasing

`main` is protected: everything goes through a pull request. To release version `0.2.0`:

1. a "Version 0.2.0" pull request that sets `version` to `0.2.0` in `Cargo.toml` (then
   `cargo update --workspace` for `Cargo.lock`);
2. once it is merged, "Run workflow" on the `Release` workflow in the Actions tab, with the
   version name (`v0.2.0`).

The workflow builds for Linux, macOS (Intel and Apple Silicon) and Windows, then publishes the
archives, `SHA256SUMS` and the install scripts. The release notes come from the pull request
titles. `v*` tags can be neither moved nor deleted.

### Code signing (Windows)

Releases are not signed. SignPath Foundation (free code signing for open source) turned the
project down in October 2026, as too new and not well known enough yet; a new application is
possible later, on [signpath.org](https://signpath.org). The workflow is ready: it has SignPath
(Foundation or a paid plan) sign the three `.exe` files, `easytab-profile.ps1` and `install.ps1`
as soon as the repository has the `SIGNPATH_ORGANIZATION_ID` variable; without it, the release
ships unsigned. To set up once:

- in SignPath: enable the "GitHub.com" trusted build system and link it to the project; the
  project's artifact configuration is `.signpath/artifact-configuration.xml`;
- in GitHub (Settings → Secrets and variables → Actions): the `SIGNPATH_API_TOKEN` secret (token
  of a SignPath user allowed to submit), and the `SIGNPATH_ORGANIZATION_ID`,
  `SIGNPATH_PROJECT_SLUG` and `SIGNPATH_POLICY_SLUG` variables.

The workflow checks every signature and rejects a signed script that would start with a BOM
(`irm | iex` refuses it).

### winget

`packaging/winget` describes the `AdamSellin.EasyTab` package (the zip, with `easytab` in the
PATH; `easytab install` still has to be run). Before submitting it, put in the version, the URL
and the SHA-256 (`Get-FileHash`) of the latest release, then run
`winget validate --manifest packaging\winget` and `wingetcreate submit packaging\winget`. Then,
for each release: `wingetcreate update AdamSellin.EasyTab --version 0.2.0 --urls <zip url>
--submit`.

[← Contents](README.md)
