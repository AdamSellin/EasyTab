//! `easytab` : installe et désinstalle l'intégration shell, et aide au diagnostic.

// Partagé avec easytab-term sans dépendre de tout easytab-core (QuickJS).
#[path = "../../easytab-core/src/config.rs"]
#[allow(dead_code)]
mod config;
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
    /// Crée le fichier de réglages s'il n'existe pas, et affiche son chemin
    Config,
}

#[derive(Clone, Copy, ValueEnum)]
enum Shell {
    Zsh,
    Bash,
    /// PowerShell 7 (`pwsh`) et Windows PowerShell 5.
    Pwsh,
}

impl Shell {
    const ALL: [Shell; 3] = [Shell::Zsh, Shell::Bash, Shell::Pwsh];

    fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Pwsh => "pwsh",
        }
    }

    fn script(self) -> &'static str {
        match self {
            Shell::Zsh => include_str!("../../../shell-integration/easytab.zsh"),
            Shell::Bash => include_str!("../../../shell-integration/easytab.bash"),
            Shell::Pwsh => include_str!("../../../shell-integration/easytab.ps1"),
        }
    }

    fn quote(self, value: &str) -> String {
        match self {
            Shell::Pwsh => rc::powershell_quote(value),
            _ => rc::shell_quote(value),
        }
    }

    /// Ligne ajoutée au fichier de config pour charger l'intégration.
    fn load_line(self, exe: &Path) -> String {
        let exe = self.quote(&exe.to_string_lossy());
        match self {
            Shell::Pwsh => format!("Invoke-Expression (& {exe} init pwsh | Out-String)"),
            _ => format!("eval \"$({exe} init {})\"", self.name()),
        }
    }

    /// Fichiers de config du shell. PowerShell : profils de PowerShell 7 et de
    /// Windows PowerShell (le dossier Documents peut être dans OneDrive).
    fn rc_files(self) -> Result<Vec<PathBuf>> {
        let home = dirs::home_dir().context("dossier personnel introuvable")?;
        Ok(match self {
            Shell::Zsh => vec![std::env::var_os("ZDOTDIR")
                .map(PathBuf::from)
                .unwrap_or(home)
                .join(".zshrc")],
            Shell::Bash => vec![home.join(".bashrc")],
            Shell::Pwsh if cfg!(windows) => {
                let documents = dirs::document_dir().unwrap_or_else(|| home.join("Documents"));
                ["PowerShell", "WindowsPowerShell"]
                    .iter()
                    .map(|dir| documents.join(dir).join("Microsoft.PowerShell_profile.ps1"))
                    .collect()
            }
            Shell::Pwsh => vec![home
                .join(".config")
                .join("powershell")
                .join("Microsoft.PowerShell_profile.ps1")],
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
                bail!(
                    "shell non pris en charge ({shell:?}) ; précise --shell zsh, --shell bash ou --shell pwsh"
                )
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
        Command::Config => edit_config()?,
    }
    Ok(())
}

fn init_script(shell: Shell) -> Result<String> {
    let term = shell.quote(&term_binary().to_string_lossy());
    Ok(shell.script().replace("__EASYTAB_TERM_BIN__", &term))
}

fn install(shell: Shell) -> Result<()> {
    let exe = copy_binaries()?;
    let line = shell.load_line(&exe);
    for path in shell.rc_files()? {
        let content = read_or_empty(&path)?;
        let updated = rc::has_block(&content);
        // Réinstaller remplace les anciens blocs par ceux de cette version.
        write_config(
            shell,
            &path,
            &rc::add_blocks(&rc::remove_blocks(&content), &line),
        )?;
        println!(
            "EasyTab est {} dans {}",
            if updated { "mis à jour" } else { "installé" },
            path.display()
        );
    }
    println!(
        "Programmes dans {}. Ouvre un nouveau terminal pour activer EasyTab.",
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
    // Fenêtre flottante (Windows) : facultative, la liste peut toujours être
    // dessinée dans le terminal.
    let overlay = exe.with_file_name(format!("easytab-overlay{}", std::env::consts::EXE_SUFFIX));
    let mut installed = None;
    for source in [&exe, &term, &overlay] {
        if !source.is_file() {
            continue;
        }
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
    for path in shell.rc_files()? {
        let content = read_or_empty(&path)?;
        if !rc::has_block(&content) {
            println!("EasyTab n'est pas installé dans {}", path.display());
            continue;
        }
        write_config(shell, &path, &rc::remove_blocks(&content))?;
        println!("EasyTab est retiré de {}", path.display());
    }
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
    if cfg!(windows) {
        let overlay =
            term.with_file_name(format!("easytab-overlay{}", std::env::consts::EXE_SUFFIX));
        println!(
            "{} fenêtre flottante : {}",
            mark(overlay.is_file()),
            if overlay.is_file() {
                overlay.display().to_string()
            } else {
                "absente, la liste s'affiche dans le terminal".to_string()
            }
        );
    }
    for shell in Shell::ALL {
        let mut installed_in = Vec::new();
        for path in shell.rc_files()? {
            if rc::has_block(&read_or_empty(&path)?) {
                installed_in.push(path.display().to_string());
            }
        }
        println!(
            "{} {} : {}",
            mark(!installed_in.is_empty()),
            shell.name(),
            if installed_in.is_empty() {
                format!("non installé (easytab install --shell {})", shell.name())
            } else {
                format!("installé dans {}", installed_in.join(", "))
            }
        );
    }
    if let Some(path) = config::Config::path() {
        let (ok, status) = config_status(&path);
        println!("{} réglages : {status}", mark(ok));
    }
    let active = std::env::var_os("EASYTAB_TERM").is_some();
    println!(
        "{} ce terminal {} sous EasyTab",
        mark(active),
        if active { "tourne" } else { "ne tourne pas" }
    );
    Ok(())
}

/// État du fichier de réglages, pour `doctor` et `config`.
fn config_status(path: &Path) -> (bool, String) {
    match config::Config::read(path) {
        Ok(Some(_)) => (true, path.display().to_string()),
        Ok(None) => (
            true,
            "par défaut (easytab config pour les changer)".to_string(),
        ),
        Err(error) => (
            false,
            format!(
                "{} est invalide, réglages par défaut utilisés :\n     {}",
                path.display(),
                error.trim().replace('\n', "\n     ")
            ),
        ),
    }
}

/// `easytab config` : crée `~/.easytab/config.toml` avec chaque réglage
/// commenté, puis dit où il est.
fn edit_config() -> Result<()> {
    let path = config::Config::path().context("dossier personnel introuvable")?;
    if !path.exists() {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
        }
        fs::write(&path, config::TEMPLATE)
            .with_context(|| format!("écriture de {}", path.display()))?;
        println!("Fichier de réglages créé : {}", path.display());
    } else {
        println!("Fichier de réglages : {}", path.display());
    }
    let (ok, status) = config_status(&path);
    if !ok {
        println!("{} {status}", mark(ok));
    }
    let editor = if cfg!(windows) {
        "notepad"
    } else {
        "${EDITOR:-nano}"
    };
    println!(
        "Modifiez-le ({editor} \"{}\"), puis rouvrez vos terminaux.",
        path.display()
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
        // Les profils PowerShell commencent souvent par un BOM UTF-8.
        Ok(content) => Ok(content.trim_start_matches('\u{feff}').to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("lecture de {}", path.display())),
    }
}

/// Écrit un fichier de config. Les profils PowerShell gardent un BOM UTF-8 :
/// sans lui, Windows PowerShell 5 les lit comme de l'ANSI.
fn write_config(shell: Shell, path: &Path, content: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    }
    let content = match shell {
        Shell::Pwsh if !content.is_empty() => format!("\u{feff}{content}"),
        _ => content.to_string(),
    };
    fs::write(path, content).with_context(|| format!("écriture de {}", path.display()))
}

fn mark(ok: bool) -> &'static str {
    if ok {
        "[ok]"
    } else {
        "[--]"
    }
}
