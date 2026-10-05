//! Historique du shell : comme Fig, EasyTab propose les commandes entières
//! déjà tapées qui commencent comme la ligne en cours
//! (`docker-compose u` → `docker-compose up -d --build`).

use std::path::{Path, PathBuf};

/// Lignes gardées (les plus récentes).
const MAX_LINES: usize = 5000;
/// Suggestions tirées de l'historique pour une ligne.
const MAX_MATCHES: usize = 3;

#[derive(Debug, Default, Clone)]
pub struct History {
    /// Commandes, de la plus ancienne à la plus récente, sans doublon.
    lines: Vec<String>,
}

/// Shells dont on sait lire l'historique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    PowerShell,
}

impl History {
    pub fn new(lines: impl IntoIterator<Item = String>) -> Self {
        let mut history = Self::default();
        for line in lines {
            history.push(&line);
        }
        let excess = history.lines.len().saturating_sub(MAX_LINES);
        history.lines.drain(..excess);
        history
    }

    /// Historique du shell, lu dans son fichier habituel.
    pub fn load(shell: Shell) -> Self {
        let Some(path) = history_file(shell) else {
            return Self::default();
        };
        let Ok(data) = std::fs::read(&path) else {
            return Self::default();
        };
        let text = String::from_utf8_lossy(&data);
        Self::new(parse(shell, &text))
    }

    /// Ajoute une commande exécutée (elle devient la plus récente).
    pub fn push(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || line.contains('\n') {
            return;
        }
        self.lines.retain(|l| l != line);
        self.lines.push(line.to_string());
    }

    /// Commandes déjà tapées qui prolongent `input`, les plus récentes d'abord.
    pub fn matches(&self, input: &str) -> Vec<&str> {
        let input = input.trim_start();
        // Trop court : presque tout correspondrait.
        if input.chars().count() < 3 {
            return Vec::new();
        }
        self.lines
            .iter()
            .rev()
            .filter(|line| line.len() > input.len() && line.starts_with(input))
            .map(String::as_str)
            .take(MAX_MATCHES)
            .collect()
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn history_file(shell: Shell) -> Option<PathBuf> {
    match shell {
        Shell::Bash => std::env::var_os("HISTFILE")
            .map(PathBuf::from)
            .or_else(|| Some(home()?.join(".bash_history"))),
        Shell::Zsh => std::env::var_os("HISTFILE")
            .map(PathBuf::from)
            .or_else(|| Some(home()?.join(".zsh_history"))),
        Shell::PowerShell => {
            // PSReadLine : %APPDATA% sous Windows, ~/.local/share ailleurs.
            let dir = match std::env::var_os("APPDATA") {
                Some(appdata) => Path::new(&appdata).join("Microsoft").join("Windows"),
                None => home()?.join(".local").join("share"),
            };
            Some(
                dir.join("PowerShell")
                    .join("PSReadLine")
                    .join("ConsoleHost_history.txt"),
            )
        }
    }
}

/// Commandes du fichier d'historique, dans l'ordre.
fn parse(shell: Shell, text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut continued = false;
    for raw in text.lines() {
        let line = match shell {
            // `HISTTIMEFORMAT` ajoute des lignes `#1696500000`.
            Shell::Bash if raw.starts_with('#') && raw[1..].chars().all(|c| c.is_ascii_digit()) => {
                continue
            }
            // Format étendu : `: 1696500000:0;git status`.
            Shell::Zsh => raw
                .strip_prefix(": ")
                .and_then(|rest| rest.split_once(';'))
                .map_or(raw, |(_, command)| command),
            _ => raw,
        };
        // Commande sur plusieurs lignes (`\` en zsh et bash, `` ` `` en
        // PowerShell) : ignorée.
        let continues = line.ends_with('\\') || line.ends_with('`');
        if !continued && !continues {
            lines.push(line.to_string());
        }
        continued = continues;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(lines: &[&str]) -> History {
        History::new(lines.iter().map(|l| l.to_string()))
    }

    #[test]
    fn suggests_recent_commands_first() {
        let history = history(&[
            "docker-compose up -d",
            "git status",
            "docker-compose up -d --build",
            "docker-compose up -d",
            "docker-compose logs -f",
        ]);
        assert_eq!(
            history.matches("docker-compose u"),
            ["docker-compose up -d", "docker-compose up -d --build"]
        );
        // La ligne tapée elle-même n'est pas proposée.
        assert!(history.matches("git status").is_empty());
        assert!(history.matches("gi").is_empty());
    }

    #[test]
    fn keeps_three_matches_at_most() {
        let history = history(&["make a", "make b", "make c", "make d"]);
        assert_eq!(history.matches("make"), ["make d", "make c", "make b"]);
    }

    #[test]
    fn records_new_commands() {
        let mut history = history(&["npm run build"]);
        history.push("npm run test");
        assert_eq!(history.matches("npm r"), ["npm run test", "npm run build"]);
    }

    #[test]
    fn reads_shell_history_files() {
        assert_eq!(
            parse(Shell::Bash, "#1696500000\ngit status\nls -la\n"),
            ["git status", "ls -la"]
        );
        assert_eq!(
            parse(Shell::Zsh, ": 1696500000:0;git push\nmake\n"),
            ["git push", "make"]
        );
        assert_eq!(
            parse(Shell::PowerShell, "Get-ChildItem\nfoo `\n  -Bar\ncd ..\n"),
            ["Get-ChildItem", "cd .."]
        );
    }
}
