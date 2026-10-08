//! Lancement des commandes demandées par les generators (`git branch`…).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Most bytes kept from each stream. A generator only needs a list of names;
/// beyond this, the rest is read and dropped so the command never blocks on
/// a full pipe.
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub status: i32,
}

/// Lance `program` dans `cwd` et renvoie sa sortie (fins de ligne `\n`, sans
/// blancs finaux). Au-delà de `timeout`, la commande est arrêtée.
pub fn run(program: &str, args: &[String], cwd: &Path, timeout: Duration) -> Output {
    let Some(path) = find_program(program) else {
        return Output {
            stderr: format!("{program}: commande introuvable"),
            status: 127,
            ..Output::default()
        };
    };
    let mut command = Command::new(path);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Pas de fenêtre de console qui clignote.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Output {
                stderr: format!("{program}: {e}"),
                status: 126,
                ..Output::default()
            }
        }
    };
    let stdout = read_in_background(child.stdout.take());
    let stderr = read_in_background(child.stderr.take());

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(1),
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break 124;
            }
        }
    };
    // Un processus lancé en arrière-plan par la commande peut garder la sortie
    // ouverte : on n'attend pas sa fin au-delà d'un court délai.
    let deadline = Instant::now() + Duration::from_millis(200);
    let collect = |pipe: mpsc::Receiver<(Vec<u8>, bool)>| {
        let (data, truncated) = pipe
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_default();
        (clean(data), truncated)
    };
    let (stdout, stdout_truncated) = collect(stdout);
    let (mut stderr, stderr_truncated) = collect(stderr);
    for (name, truncated) in [("stdout", stdout_truncated), ("stderr", stderr_truncated)] {
        if truncated {
            if !stderr.is_empty() {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "easytab: {program}: {name} truncated after {} MiB",
                MAX_OUTPUT / (1024 * 1024)
            ));
        }
    }
    Output {
        stdout,
        stderr,
        status,
    }
}

fn read_in_background(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<(Vec<u8>, bool)> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let read = pipe.map(|pipe| read_capped(pipe, MAX_OUTPUT));
        let _ = sender.send(read.unwrap_or_default());
    });
    receiver
}

/// Reads `reader` to the end but keeps at most `limit` bytes, cut after the
/// last full line. The flag tells whether something was dropped.
fn read_capped(mut reader: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut data = Vec::new();
    let _ = reader.by_ref().take(limit as u64).read_to_end(&mut data);
    let mut buf = [0u8; 16 * 1024];
    let mut truncated = false;
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        truncated = true;
    }
    if truncated {
        // A cut name would show up as a wrong suggestion.
        let end = data.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        data.truncate(end);
    }
    (data, truncated)
}

fn clean(data: Vec<u8>) -> String {
    String::from_utf8_lossy(&data)
        .replace("\r\n", "\n")
        .trim_end()
        .to_string()
}

/// Cherche le programme dans le PATH. On ne laisse pas Windows le faire : il
/// regarde d'abord dans `System32`, où `bash.exe` est celui de WSL.
fn find_program(program: &str) -> Option<PathBuf> {
    if program.contains(['/', '\\']) {
        return Some(PathBuf::from(program));
    }
    let found = search_path(program);
    // `bash -c` sur un système qui n'a que `sh`.
    if found.is_none() && program == "bash" {
        return search_path("sh");
    }
    found
}

/// Chemin du programme `program` dans le PATH (avec son extension sous Windows).
pub(crate) fn find_in_path(program: &str) -> Option<PathBuf> {
    search_path(program)
}

fn search_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let mut extensions = executable_extensions();
    // `pwsh.exe` : le nom est déjà complet.
    if Path::new(program).extension().is_some() && !extensions.contains(&String::new()) {
        extensions.insert(0, String::new());
    }
    std::env::split_paths(&path)
        .flat_map(|dir| {
            extensions
                .iter()
                .map(move |ext| dir.join(format!("{program}{ext}")))
        })
        .find(|candidate| candidate.is_file())
}

/// Noms des programmes du PATH (sans `.exe` sous Windows), pour ne proposer
/// que des commandes installées.
pub fn installed_programs() -> std::collections::HashSet<String> {
    let extensions = executable_extensions();
    let Some(path) = std::env::var_os("PATH") else {
        return Default::default();
    };
    let mut names = std::collections::HashSet::new();
    for dir in std::env::split_paths(&path) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if cfg!(windows) {
                let lower = name.to_ascii_lowercase();
                if let Some(ext) = extensions
                    .iter()
                    .find(|ext| !ext.is_empty() && lower.ends_with(ext.as_str()))
                {
                    names.insert(lower[..lower.len() - ext.len()].to_string());
                }
            } else {
                names.insert(name);
            }
        }
    }
    names
}

#[cfg(windows)]
fn executable_extensions() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_lowercase())
        .collect()
}

#[cfg(not(windows))]
fn executable_extensions() -> Vec<String> {
    vec![String::new()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_long_output_at_a_line_end() {
        let input = b"main\nfeature\nfix-overflow\n".repeat(1000);
        let (data, truncated) = read_capped(&input[..], 30);
        assert!(truncated);
        assert_eq!(data, b"main\nfeature\nfix-overflow\n");

        let (data, truncated) = read_capped(&b"main\nfeature\n"[..], 30);
        assert!(!truncated);
        assert_eq!(data, b"main\nfeature\n");

        // Exactly at the limit: nothing dropped.
        let (data, truncated) = read_capped(&b"abc\n"[..], 4);
        assert!(!truncated);
        assert_eq!(data, b"abc\n");

        // Huge stream, no line end: bounded memory, nothing kept.
        let (data, truncated) = read_capped(std::io::repeat(b'x').take(10 * 1024 * 1024), 1024);
        assert!(truncated);
        assert!(data.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn reports_truncated_output() {
        let output = run(
            "sh",
            &["-c".into(), "yes main | head -c 6000000".into()],
            Path::new("/"),
            Duration::from_secs(10),
        );
        assert_eq!(output.status, 0);
        assert!(output.stdout.len() <= MAX_OUTPUT);
        assert!(output.stdout.ends_with("main"));
        assert!(output.stderr.contains("stdout truncated"));
    }

    #[cfg(unix)]
    #[test]
    fn captures_output_and_status() {
        let output = run(
            "sh",
            &[
                "-c".into(),
                "printf 'a\\r\\nb\\n\\n'; echo err >&2; exit 3".into(),
            ],
            Path::new("/"),
            Duration::from_secs(5),
        );
        assert_eq!(output.stdout, "a\nb");
        assert_eq!(output.stderr, "err");
        assert_eq!(output.status, 3);
    }

    #[cfg(unix)]
    #[test]
    fn stops_slow_commands() {
        let start = Instant::now();
        let output = run(
            "sh",
            &["-c".into(), "sleep 5".into()],
            Path::new("/"),
            Duration::from_millis(100),
        );
        assert_eq!(output.status, 124);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn reports_missing_programs() {
        let output = run(
            "easytab-inexistant",
            &[],
            Path::new("/"),
            Duration::from_secs(1),
        );
        assert_eq!(output.status, 127);
    }
}
