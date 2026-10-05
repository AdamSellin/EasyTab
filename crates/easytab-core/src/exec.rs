//! Lancement des commandes demandées par les generators (`git branch`…).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

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
    let collect = |pipe: mpsc::Receiver<Vec<u8>>| {
        clean(
            pipe.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_default(),
        )
    };
    Output {
        stdout: collect(stdout),
        stderr: collect(stderr),
        status,
    }
}

fn read_in_background(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut data = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut data);
        }
        let _ = sender.send(data);
    });
    receiver
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

fn search_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions = executable_extensions();
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

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
