//! `easytab` : installe et désinstalle l'intégration shell, et aide au diagnostic.

// Partagé avec easytab-term sans dépendre de tout easytab-core (QuickJS).
#[path = "../../easytab-core/src/config.rs"]
#[allow(dead_code)]
mod config;
// Langue des messages, partagée de la même façon.
#[path = "../../easytab-core/src/lang.rs"]
mod lang;
mod rc;
mod update;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use lang::tr;

// Les textes d'aide sont des attributs plutôt que des commentaires `///`,
// pour suivre la langue de l'utilisateur.
#[derive(Parser)]
#[command(
    version,
    about = tr("Graphical autocomplete for the terminal", "Autocomplétion graphique pour le terminal")
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = tr(
        "Print a shell's integration script (used by the shell's config file)",
        "Affiche le script d'intégration d'un shell (utilisé par le fichier de config du shell)"
    ))]
    Init { shell: Shell },
    #[command(about = tr(
        "Enable EasyTab in the shell's config file",
        "Active EasyTab dans le fichier de config du shell"
    ))]
    Install {
        #[arg(long, help = shell_help())]
        shell: Option<Shell>,
    },
    #[command(about = tr(
        "Remove EasyTab from the shell's config file",
        "Retire EasyTab du fichier de config du shell"
    ))]
    Uninstall {
        #[arg(long, help = shell_help())]
        shell: Option<Shell>,
    },
    #[command(about = tr("Check the installation", "Vérifie l'installation"))]
    Doctor,
    #[command(about = tr(
        "Create the settings file if it does not exist, and print its path",
        "Crée le fichier de réglages s'il n'existe pas, et affiche son chemin"
    ))]
    Config,
    #[command(about = tr("Install the latest published version", "Installe la dernière version publiée"))]
    Update {
        #[arg(long, help = tr(
            "Reinstall even if the version is already the latest",
            "Réinstalle même si la version est déjà la dernière"
        ))]
        force: bool,
    },
}

fn shell_help() -> &'static str {
    tr(
        "Shell to configure (default: the one in $SHELL)",
        "Shell à configurer (par défaut : celui de $SHELL)",
    )
}

#[derive(Clone, Copy, ValueEnum)]
enum Shell {
    Zsh,
    Bash,
    #[value(help = tr(
        "PowerShell 7 (`pwsh`) and Windows PowerShell 5",
        "PowerShell 7 (`pwsh`) et Windows PowerShell 5"
    ))]
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
        let home = dirs::home_dir().context(home_not_found())?;
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
                bail!(tr!(
                    "unsupported shell ({shell:?}); pass --shell zsh, --shell bash or --shell pwsh",
                    "shell non pris en charge ({shell:?}) ; précise --shell zsh, --shell bash ou --shell pwsh"
                ))
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
        Command::Update { force } => update(force)?,
    }
    Ok(())
}

/// Met à jour en gardant les shells déjà configurés.
fn update(force: bool) -> Result<()> {
    let mut installed = Vec::new();
    for shell in Shell::ALL {
        for path in shell.rc_files()? {
            if rc::has_block(&read_or_empty(&path)?) {
                installed.push(shell.name());
                break;
            }
        }
    }
    update::run(force, &installed)
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
        let path = short(&path);
        println!(
            "{}",
            if updated {
                tr!(
                    "EasyTab updated in {path}",
                    "EasyTab est mis à jour dans {path}"
                )
            } else {
                tr!(
                    "EasyTab installed in {path}",
                    "EasyTab est installé dans {path}"
                )
            }
        );
    }
    let dir = short(exe.parent().unwrap_or(&exe));
    println!(
        "{}",
        tr!(
            "Programs in {dir}. Open a new terminal to enable EasyTab.",
            "Programmes dans {dir}. Ouvre un nouveau terminal pour activer EasyTab."
        )
    );
    Ok(())
}

fn home_not_found() -> &'static str {
    tr("home directory not found", "dossier personnel introuvable")
}

/// Contexte d'erreur « création de <dossier> ».
fn creating(path: &Path) -> String {
    let path = path.display();
    tr!("creating {path}", "création de {path}")
}

/// Contexte d'erreur « écriture de <fichier> ».
fn writing(path: &Path) -> String {
    let path = path.display();
    tr!("writing {path}", "écriture de {path}")
}

/// Dossier où `install` copie les programmes : le fichier de config du shell
/// pointe vers lui plutôt que vers le dossier de compilation.
fn install_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context(home_not_found())?;
    Ok(home.join(".easytab").join("bin"))
}

/// Copie `easytab` et `easytab-term` dans [`install_dir`] ; renvoie le chemin
/// de `easytab` installé.
fn copy_binaries() -> Result<PathBuf> {
    let exe = std::env::current_exe().context(tr(
        "path of easytab not found",
        "chemin de easytab introuvable",
    ))?;
    let dir = install_dir()?;
    if exe.parent() == Some(dir.as_path()) {
        return Ok(exe);
    }
    let term = term_binary();
    if !term.is_file() {
        let (term, exe) = (term.display(), exe.display());
        bail!(tr!(
            "{term} not found next to {exe}: build the whole project (cargo build --release)",
            "{term} introuvable à côté de {exe} : compile tout le projet (cargo build --release)"
        ));
    }
    fs::create_dir_all(&dir).with_context(|| creating(&dir))?;
    remove_old_copies(&dir);
    // Fenêtre flottante : facultative, la liste peut toujours être
    // dessinée dans le terminal.
    let overlay = exe.with_file_name(format!("easytab-overlay{}", std::env::consts::EXE_SUFFIX));
    let mut installed = None;
    for source in [&exe, &term, &overlay] {
        if !source.is_file() {
            continue;
        }
        let name = source
            .file_name()
            .context(tr("invalid program name", "nom de programme invalide"))?;
        let target = dir.join(name);
        replace_file(source, &target)?;
        if source == &exe {
            installed = Some(target);
        }
    }
    installed.context(tr("copying easytab", "copie de easytab"))
}

/// Remplace `target` par une copie de `source`. Sous Windows, un programme en
/// cours d'exécution (dans un terminal ouvert) ne peut pas être effacé, mais
/// peut être renommé : l'ancienne version est mise de côté.
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    if target.exists() && fs::remove_file(target).is_err() {
        let mut aside = target.as_os_str().to_owned();
        aside.push(format!(".old-{}", std::process::id()));
        fs::rename(target, &aside).with_context(|| {
            let target = target.display();
            tr!("{target} is locked", "{target} est verrouillé")
        })?;
    }
    fs::copy(source, target).with_context(|| {
        let (source, target) = (source.display(), target.display());
        tr!(
            "copying {source} to {target}",
            "copie de {source} vers {target}"
        )
    })?;
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
        let short_path = short(&path);
        if !rc::has_block(&content) {
            println!(
                "{}",
                tr!(
                    "EasyTab is not installed in {short_path}",
                    "EasyTab n'est pas installé dans {short_path}"
                )
            );
            continue;
        }
        write_config(shell, &path, &rc::remove_blocks(&content))?;
        println!(
            "{}",
            tr!(
                "EasyTab removed from {short_path}",
                "EasyTab est retiré de {short_path}"
            )
        );
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
        "{} wrapper easytab-term{}{}",
        mark(term_found),
        colon(),
        short(&term)
    );
    if cfg!(any(windows, target_os = "macos", target_os = "linux")) {
        let overlay =
            term.with_file_name(format!("easytab-overlay{}", std::env::consts::EXE_SUFFIX));
        println!(
            "{} {}{}{}",
            mark(overlay.is_file()),
            tr("floating window", "fenêtre flottante"),
            colon(),
            if overlay.is_file() {
                short(&overlay)
            } else {
                tr(
                    "missing, the list is drawn in the terminal",
                    "absente, la liste s'affiche dans le terminal",
                )
                .to_string()
            }
        );
    }
    for shell in Shell::ALL {
        let mut installed_in = Vec::new();
        for path in shell.rc_files()? {
            if rc::has_block(&read_or_empty(&path)?) {
                installed_in.push(short(&path));
            }
        }
        let name = shell.name();
        println!(
            "{} {name}{}{}",
            mark(!installed_in.is_empty()),
            colon(),
            if installed_in.is_empty() {
                tr!(
                    "not installed (easytab install --shell {name})",
                    "non installé (easytab install --shell {name})"
                )
            } else {
                let files = installed_in.join(", ");
                tr!("installed in {files}", "installé dans {files}")
            }
        );
    }
    if let Some(path) = config::Config::path() {
        let (ok, status) = config_status(&path);
        println!(
            "{} {}{}{status}",
            mark(ok),
            tr("settings", "réglages"),
            colon()
        );
    }
    let active = std::env::var_os("EASYTAB_TERM").is_some();
    println!(
        "{} {}",
        mark(active),
        if active {
            tr(
                "this terminal runs under EasyTab",
                "ce terminal tourne sous EasyTab",
            )
        } else {
            tr(
                "this terminal does not run under EasyTab",
                "ce terminal ne tourne pas sous EasyTab",
            )
        }
    );
    Ok(())
}

/// État du fichier de réglages, pour `doctor` et `config`.
fn config_status(path: &Path) -> (bool, String) {
    match config::Config::read(path) {
        Ok(Some(_)) => (true, short(path)),
        Ok(None) => (
            true,
            tr(
                "defaults (easytab config to change them)",
                "par défaut (easytab config pour les changer)",
            )
            .to_string(),
        ),
        Err(error) => {
            let path = short(path);
            let error = error.trim().replace('\n', "\n     ");
            (
                false,
                tr!(
                    "{path} is invalid, default settings used:\n     {error}",
                    "{path} est invalide, réglages par défaut utilisés :\n     {error}"
                ),
            )
        }
    }
}

/// `easytab config` : crée `~/.easytab/config.toml` avec chaque réglage
/// commenté, puis dit où il est.
fn edit_config() -> Result<()> {
    let path = config::Config::path().context(home_not_found())?;
    let short_path = short(&path);
    if !path.exists() {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| creating(dir))?;
        }
        fs::write(&path, config::template()).with_context(|| writing(&path))?;
        println!(
            "{}",
            tr!(
                "Settings file created: {short_path}",
                "Fichier de réglages créé : {short_path}"
            )
        );
    } else {
        println!(
            "{}",
            tr!(
                "Settings file: {short_path}",
                "Fichier de réglages : {short_path}"
            )
        );
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
    let path = path.display();
    println!(
        "{}",
        tr!(
            "Edit it ({editor} \"{path}\"), then reopen your terminals.",
            "Modifiez-le ({editor} \"{path}\"), puis rouvrez vos terminaux."
        )
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
        Err(e) => Err(e).with_context(|| {
            let path = path.display();
            tr!("reading {path}", "lecture de {path}")
        }),
    }
}

/// Écrit un fichier de config. Les profils PowerShell gardent un BOM UTF-8 :
/// sans lui, Windows PowerShell 5 les lit comme de l'ANSI.
fn write_config(shell: Shell, path: &Path, content: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| creating(dir))?;
    }
    let content = match shell {
        Shell::Pwsh if !content.is_empty() => format!("\u{feff}{content}"),
        _ => content.to_string(),
    };
    fs::write(path, content).with_context(|| writing(path))
}

/// Deux-points, précédé d'une espace en français.
fn colon() -> &'static str {
    tr(": ", " : ")
}

fn mark(ok: bool) -> &'static str {
    if ok {
        "[ok]"
    } else {
        "[--]"
    }
}

/// Chemin à afficher : `~` à la place du dossier personnel, pour des messages
/// plus courts qui ne montrent pas le nom d'utilisateur.
fn short(path: &Path) -> String {
    match dirs::home_dir() {
        Some(home) => short_in(path, &home),
        None => path.display().to_string(),
    }
}

fn short_in(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_replaces_home_with_tilde() {
        let home = Path::new("/home/adam");
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            short_in(&home.join(".easytab").join("bin"), home),
            format!("~{sep}.easytab{sep}bin")
        );
        assert_eq!(short_in(home, home), "~");
        assert_eq!(short_in(Path::new("/etc/profile"), home), "/etc/profile");
        assert_eq!(
            short_in(Path::new("/home/adamx/.bashrc"), home),
            "/home/adamx/.bashrc"
        );
    }
}
