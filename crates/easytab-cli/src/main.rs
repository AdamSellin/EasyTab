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
    let exe = copy_binaries()?;
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
        "EasyTab est {} dans {} (programmes dans {}). Ouvre un nouveau terminal pour l'activer.",
        if updated { "mis à jour" } else { "installé" },
        path.display(),
        exe.parent().unwrap_or(&exe).display()
    );
    Ok(())
}

/// Dossier où `install` copie les programmes : le fichier de config du shell
/// pointe vers lui plutôt que vers le dossier de compilation.
fn install_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("dossier personnel introuvable")?;
    Ok(home.join(".easytab").join("bin"))
}

/// Copie `easytab` et `easytab-term` dans [`install_dir`] ; renvoie le chemin
/// de `easytab` installé.
fn copy_binaries() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("chemin de easytab introuvable")?;
    let dir = install_dir()?;
    if exe.parent() == Some(dir.as_path()) {
        return Ok(exe);
    }
    let term = term_binary();
    if !term.is_file() {
        bail!(
            "{} introuvable à côté de {} : compile tout le projet (cargo build --release)",
            term.display(),
            exe.display()
        );
    }
    fs::create_dir_all(&dir).with_context(|| format!("création de {}", dir.display()))?;
    remove_old_copies(&dir);
    let mut installed = None;
    for source in [&exe, &term] {
        let name = source.file_name().context("nom de programme invalide")?;
        let target = dir.join(name);
        replace_file(source, &target)?;
        if source == &exe {
            installed = Some(target);
        }
    }
    installed.context("copie de easytab")
}

/// Remplace `target` par une copie de `source`. Sous Windows, un programme en
/// cours d'exécution (dans un terminal ouvert) ne peut pas être effacé, mais
/// peut être renommé : l'ancienne version est mise de côté.
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    if target.exists() && fs::remove_file(target).is_err() {
        let mut aside = target.as_os_str().to_owned();
        aside.push(format!(".old-{}", std::process::id()));
        fs::rename(target, &aside)
            .with_context(|| format!("{} est verrouillé", target.display()))?;
    }
    fs::copy(source, target)
        .with_context(|| format!("copie de {} vers {}", source.display(), target.display()))?;
    Ok(())
}

/// Efface les anciennes versions mises de côté, quand plus rien ne les utilise.
fn remove_old_copies(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().contains(".old-") {
            let _ = fs::remove_file(entry.path());
        }
    }
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
    // La copie installée est celle que lance le shell, pas celle d'à côté.
    let name = format!("easytab-term{}", std::env::consts::EXE_SUFFIX);
    let term = install_dir()
        .map(|dir| dir.join(&name))
        .ok()
        .filter(|path| path.is_file())
        .unwrap_or_else(term_binary);
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
