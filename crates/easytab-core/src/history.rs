//! Historique du shell : comme Fig, EasyTab propose les commandes entières
//! déjà tapées qui commencent comme la ligne en cours
//! (`docker-compose u` → `docker-compose up -d --build`).
//!
//! Comme Atuin, EasyTab retient aussi, dans `~/.easytab/history.jsonl`, le
//! dossier, le code de sortie et la durée de chaque commande lancée sous lui :
//! les commandes lancées dans le dossier courant passent devant, celles qui
//! ont échoué derrière, et Ctrl+R cherche dans tout l'historique.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::lang::tr;

/// Lignes gardées (les plus récentes).
const MAX_LINES: usize = 5000;
/// Suggestions tirées de l'historique pour une ligne.
const MAX_MATCHES: usize = 3;
/// Résultats d'une recherche (Ctrl+R).
const MAX_SEARCH: usize = 200;
/// Dossiers retenus par commande.
const MAX_DIRS: usize = 8;

/// Une commande de l'historique.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub line: String,
    /// Dossiers où elle a été lancée, le plus récent à la fin.
    pub dirs: Vec<PathBuf>,
    /// Code de sortie de sa dernière exécution.
    pub exit_code: Option<i32>,
    pub duration: Option<Duration>,
    /// Date de sa dernière exécution, en secondes depuis 1970.
    pub time: Option<u64>,
}

impl Entry {
    fn ran_in(&self, cwd: Option<&Path>) -> bool {
        cwd.is_some_and(|cwd| self.dirs.iter().any(|dir| dir == cwd))
    }

    fn failed(&self) -> bool {
        self.exit_code.is_some_and(|code| code != 0)
    }

    /// Dossier, échec, durée et date, pour la description dans la liste :
    /// `~/projet · échec (127) · 2,5 s · il y a 3 min`.
    pub fn describe(&self, home: Option<&Path>, now: u64) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(dir) = self.dirs.last() {
            parts.push(short_path(dir, home));
        }
        if let Some(code) = self.exit_code.filter(|&code| code != 0) {
            parts.push(format!("{} ({code})", tr("failed", "échec")));
        }
        if let Some(duration) = self.duration.filter(|d| d.as_millis() >= 1000) {
            parts.push(format_duration(duration));
        }
        if let Some(time) = self.time {
            parts.push(format_age(now.saturating_sub(time)));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// Commande terminée, telle qu'enregistrée dans `~/.easytab/history.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// Durée en millisecondes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    #[serde(default)]
    pub time: u64,
}

impl Record {
    pub fn new(command: &str, cwd: Option<&Path>, exit: Option<i32>, duration: Duration) -> Self {
        Self {
            command: command.trim().to_string(),
            cwd: cwd.map(Path::to_path_buf),
            exit,
            ms: Some(duration.as_millis() as u64),
            time: now(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct History {
    /// Commandes, de la plus ancienne à la plus récente, sans doublon.
    entries: Vec<Entry>,
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
        history.truncate();
        history
    }

    fn truncate(&mut self) {
        let excess = self.entries.len().saturating_sub(MAX_LINES);
        self.entries.drain(..excess);
    }

    /// Historique du shell, lu dans son fichier habituel, complété par celui
    /// d'EasyTab (dossier, code de sortie, durée).
    pub fn load(shell: Shell) -> Self {
        let text = history_file(shell)
            .and_then(|path| std::fs::read(path).ok())
            .map(|data| String::from_utf8_lossy(&data).into_owned())
            .unwrap_or_default();
        let mut history = Self::new(parse(shell, &text));
        for record in read_records() {
            history.record(&record);
        }
        history.truncate();
        history
    }

    /// Ajoute une commande exécutée (elle devient la plus récente).
    pub fn push(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || line.contains('\n') {
            return;
        }
        let entry = match self.entries.iter().position(|e| e.line == line) {
            Some(i) => self.entries.remove(i),
            None => Entry {
                line: line.to_string(),
                ..Entry::default()
            },
        };
        self.entries.push(entry);
    }

    /// Ajoute une commande terminée, avec son dossier et son code de sortie.
    pub fn record(&mut self, record: &Record) {
        self.push(&record.command);
        let Some(entry) = self
            .entries
            .last_mut()
            .filter(|e| e.line == record.command.trim())
        else {
            return;
        };
        if let Some(cwd) = &record.cwd {
            entry.dirs.retain(|dir| dir != cwd);
            entry.dirs.push(cwd.clone());
            let excess = entry.dirs.len().saturating_sub(MAX_DIRS);
            entry.dirs.drain(..excess);
        }
        entry.exit_code = record.exit;
        entry.duration = record.ms.map(Duration::from_millis);
        entry.time = Some(record.time).filter(|&t| t > 0);
    }

    /// Commandes déjà tapées qui prolongent `input`, les plus récentes
    /// d'abord ; celles lancées dans `cwd` avant les autres, celles qui ont
    /// échoué après.
    pub fn matches(&self, input: &str, cwd: Option<&Path>) -> Vec<&str> {
        let input = input.trim_start();
        // Trop court : presque tout correspondrait.
        if input.chars().count() < 3 {
            return Vec::new();
        }
        let mut found: Vec<&Entry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| e.line.len() > input.len() && e.line.starts_with(input))
            .collect();
        // Tri stable : à égalité, la plus récente reste devant.
        found.sort_by_key(|e| (!e.ran_in(cwd), e.failed()));
        found
            .into_iter()
            .take(MAX_MATCHES)
            .map(|e| e.line.as_str())
            .collect()
    }

    /// Recherche de Ctrl+R : les commandes qui contiennent chaque mot de
    /// `query`, sans tenir compte de la casse, les plus récentes d'abord.
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.entries
            .iter()
            .rev()
            .filter(|e| {
                let line = e.line.to_lowercase();
                words.iter().all(|w| line.contains(w.as_str()))
            })
            .take(MAX_SEARCH)
            .collect()
    }
}

/// `~/.easytab/history.jsonl`.
fn records_file() -> Option<PathBuf> {
    Some(home()?.join(".easytab").join("history.jsonl"))
}

fn read_records() -> Vec<Record> {
    let Some(path) = records_file() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    // Le fichier ne fait que grandir : au-delà du double de ce qui est
    // gardé, on le réécrit avec les plus récentes.
    if lines.len() > 2 * MAX_LINES {
        let kept = lines[lines.len() - MAX_LINES..].join("\n") + "\n";
        let _ = std::fs::write(&path, kept);
    }
    lines
        .iter()
        .rev()
        .take(MAX_LINES)
        .rev()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Ajoute une commande terminée à `~/.easytab/history.jsonl`.
pub fn save(record: &Record) {
    let Some(path) = records_file() else {
        return;
    };
    let Ok(mut line) = serde_json::to_string(record) else {
        return;
    };
    line.push('\n');
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Secondes depuis 1970.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn short_path(dir: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| dir.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()).replace('\\', "/"),
        None => dir.display().to_string(),
    }
}

fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs_f64();
    if secs < 60.0 {
        let text = format!("{secs:.1} s");
        if crate::lang::fr() {
            text.replace('.', ",")
        } else {
            text
        }
    } else if secs < 3600.0 {
        format!("{} min {} s", (secs / 60.0) as u64, secs as u64 % 60)
    } else {
        format!(
            "{} h {} min",
            (secs / 3600.0) as u64,
            (secs as u64 / 60) % 60
        )
    }
}

fn format_age(secs: u64) -> String {
    let (n, en, fr) = match secs {
        0..60 => return tr("just now", "à l'instant").to_string(),
        60..3600 => (secs / 60, "min", "min"),
        3600..86400 => (secs / 3600, "h", "h"),
        _ => (secs / 86400, "d", "j"),
    };
    crate::tr!("{n} {en} ago", "il y a {n} {fr}")
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
            history.matches("docker-compose u", None),
            ["docker-compose up -d", "docker-compose up -d --build"]
        );
        // La ligne tapée elle-même n'est pas proposée.
        assert!(history.matches("git status", None).is_empty());
        assert!(history.matches("gi", None).is_empty());
    }

    #[test]
    fn keeps_three_matches_at_most() {
        let history = history(&["make a", "make b", "make c", "make d"]);
        assert_eq!(
            history.matches("make", None),
            ["make d", "make c", "make b"]
        );
    }

    #[test]
    fn records_new_commands() {
        let mut history = history(&["npm run build"]);
        history.push("npm run test");
        assert_eq!(
            history.matches("npm r", None),
            ["npm run test", "npm run build"]
        );
    }

    fn record(command: &str, cwd: &str, exit: i32) -> Record {
        Record {
            command: command.to_string(),
            cwd: Some(PathBuf::from(cwd)),
            exit: Some(exit),
            ms: Some(2500),
            time: 1000,
        }
    }

    #[test]
    fn prefers_the_current_folder_and_successes() {
        let mut history = history(&[]);
        history.record(&record("npm run dev", "/app", 0));
        history.record(&record("npm run build", "/site", 0));
        history.record(&record("npm run lint", "/app", 1));
        history.record(&record("npm run test", "/site", 2));
        assert_eq!(
            history.matches("npm r", Some(Path::new("/app"))),
            ["npm run dev", "npm run lint", "npm run build"]
        );
        assert_eq!(
            history.matches("npm r", None),
            ["npm run build", "npm run dev", "npm run test"]
        );
        // Relancée sans échec, elle repasse devant.
        history.record(&record("npm run lint", "/app", 0));
        assert_eq!(
            history.matches("npm r", Some(Path::new("/app")))[0],
            "npm run lint"
        );
    }

    #[test]
    fn keeps_metadata_when_the_shell_file_repeats_a_command() {
        let mut history = history(&["ls"]);
        history.record(&record("cargo build", "/app", 101));
        history.push("cargo build");
        let entry = history.search("cargo")[0];
        assert_eq!(entry.exit_code, Some(101));
        assert_eq!(entry.dirs, [PathBuf::from("/app")]);
    }

    #[test]
    fn searches_every_word_anywhere() {
        let history = history(&["docker compose up -d", "git status", "docker ps -a"]);
        let lines = |query| -> Vec<String> {
            history
                .search(query)
                .iter()
                .map(|e| e.line.clone())
                .collect()
        };
        assert_eq!(lines("DOCK"), ["docker ps -a", "docker compose up -d"]);
        assert_eq!(lines("up dock"), ["docker compose up -d"]);
        assert_eq!(lines("").len(), 3);
    }

    #[test]
    fn describes_an_entry() {
        let entry = Entry {
            line: "make".into(),
            dirs: vec![PathBuf::from("/home/adam/projet")],
            exit_code: Some(2),
            duration: Some(Duration::from_millis(2500)),
            time: Some(1000),
        };
        let text = entry
            .describe(Some(Path::new("/home/adam")), 1000 + 180)
            .unwrap();
        assert!(text.starts_with("~/projet · "), "{text}");
        assert!(
            text.contains("(2)") && text.contains("2") && text.contains('3'),
            "{text}"
        );
        assert_eq!(Entry::default().describe(None, 0), None);
    }

    #[test]
    fn records_are_json_lines() {
        let record = record("git push", "/app", 0);
        let json = serde_json::to_string(&record).unwrap();
        assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), record);
        let old: Record = serde_json::from_str(r#"{"command":"ls"}"#).unwrap();
        assert_eq!(old.cwd, None);
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
