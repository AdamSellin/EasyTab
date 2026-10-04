//! `easytab` : installe et désinstalle l'intégration shell, et aide au diagnostic.

mod rc;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(version, about = "Autocomplétion graphique pour le terminal")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Affiche le script d'intégration d'un shell (utilisé par le fichier de config du shell)
    Init { shell: Shell },
    /// Active EasyTab dans le fichier de config du shell
    Install {
        /// Shell à configurer (par défaut : celui de $SHELL)
        #[arg(long)]
        shell: Option<Shell>,
    },
    /// Retire EasyTab du fichier de config du shell
    Uninstall {
        /// Shell à configurer (par défaut : celui de $SHELL)
        #[arg(long)]
        shell: Option<Shell>,
    },
    /// Vérifie l'installation
    Doctor,
}

#[derive(Clone, Copy, ValueEnum)]
enum Shell {
    Zsh,
    Bash,
}

impl Shell {
    const ALL: [Shell; 2] = [Shell::Zsh, Shell::Bash];

    fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
        }
    }

    fn script(self) -> &'static str {
        match self {
            Shell::Zsh => include_str!("../../../shell-integration/easytab.zsh"),
            Shell::Bash => include_str!("../../../shell-integration/easytab.bash"),
        }
    }

    fn rc_file(self) -> Result<PathBuf> {
        let home = dirs::home_dir().context("dossier personnel introuvable")?;
        Ok(match self {
            Shell::Zsh => std::env::var_os("ZDOTDIR")
                .map(PathBuf::from)
                .unwrap_or(home)
                .join(".zshrc"),
            Shell::Bash => home.join(".bashrc"),
        })
    }

    fn detect() -> Result<Shell> {
        let shell = std::env::var("SHELL").unwrap_or_default();
        let name = Path::new(&shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        match Shell::ALL.into_iter().find(|s| s.name() == name) {
            Some(shell) => Ok(shell),
            None => {
                bail!("shell non pris en charge ({shell:?}) ; précise --shell zsh ou --shell bash")
            }
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Init { shell } => print!("{}", init_script(shell)?),
        Command::Install { shell } => install(shell.map_or_else(Shell::detect, Ok)?)?,
        Command::Uninstall { shell } => uninstall(shell.map_or_else(Shell::detect, Ok)?)?,
        Command::Doctor => doctor()?,
    }
    Ok(())
}

fn init_script(shell: Shell) -> Result<String> {
    let term = rc::shell_quote(&term_binary().to_string_lossy());
    Ok(shell.script().replace("__EASYTAB_TERM_BIN__", &term))
}

fn install(shell: Shell) -> Result<()> {
    let exe = std::env::current_exe().context("chemin de easytab introuvable")?;
    let line = format!(
        "eval \"$({} init {})\"",
        rc::shell_quote(&exe.to_string_lossy()),
        shell.name()
    );
    let path = shell.rc_file()?;
    let content = read_or_empty(&path)?;
    let updated = rc::has_block(&content);
    // Réinstaller remplace les anciens blocs par ceux de cette version.
    fs::write(&path, rc::add_blocks(&rc::remove_blocks(&content), &line))
        .with_context(|| format!("écriture de {}", path.display()))?;
    println!(
        "EasyTab est {} dans {}. Ouvre un nouveau terminal pour l'activer.",
        if updated { "mis à jour" } else { "installé" },
        path.display()
    );
    Ok(())
}

fn uninstall(shell: Shell) -> Result<()> {
    let path = shell.rc_file()?;
    let content = read_or_empty(&path)?;
    if !rc::has_block(&content) {
        println!("EasyTab n'est pas installé dans {}", path.display());
        return Ok(());
    }
    fs::write(&path, rc::remove_blocks(&content))
        .with_context(|| format!("écriture de {}", path.display()))?;
    println!("EasyTab est retiré de {}", path.display());
    Ok(())
}

fn doctor() -> Result<()> {
    let term = term_binary();
    let term_found = term.is_file() || which(&term).is_some();
    println!(
        "{} wrapper easytab-term : {}",
        mark(term_found),
        term.display()
    );
    for shell in Shell::ALL {
        let path = shell.rc_file()?;
        let installed = rc::has_block(&read_or_empty(&path)?);
        println!(
            "{} {} : {}",
            mark(installed),
            shell.name(),
            if installed {
                format!("installé dans {}", path.display())
            } else {
                format!("non installé (easytab install --shell {})", shell.name())
            }
        );
    }
    let active = std::env::var_os("EASYTAB_TERM").is_some();
    println!(
        "{} ce terminal {} sous EasyTab",
        mark(active),
        if active { "tourne" } else { "ne tourne pas" }
    );
    Ok(())
}

/// `easytab-term` installé à côté de `easytab`, sinon cherché dans le PATH.
fn term_binary() -> PathBuf {
    let name = format!("easytab-term{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn which(name: &Path) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

fn read_or_empty(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("lecture de {}", path.display())),
    }
}

fn mark(ok: bool) -> &'static str {
    if ok {
        "[ok]"
    } else {
        "[--]"
    }
}
