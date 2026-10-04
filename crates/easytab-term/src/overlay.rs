//! Fenêtre flottante façon Fig (`easytab-overlay`) : un programme à part,
//! lancé au démarrage, qui reçoit la liste à afficher sur son entrée. Le
//! clavier reste géré ici ; la fenêtre ne fait qu'afficher. Si elle ne peut
//! pas servir (programme absent, curseur introuvable), la liste est dessinée
//! dans le terminal.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;

use easytab_core::overlay::{Event, Request, View};

/// `EASYTAB_OVERLAY=0` garde la liste dans le terminal, `1` force la fenêtre
/// (sous macOS et Linux, où elle n'est pas encore prête).
const OVERLAY_ENV: &str = "EASYTAB_OVERLAY";

pub struct Overlay {
    child: Child,
    stdin: ChildStdin,
    ready: Arc<AtomicBool>,
    /// Le curseur était introuvable : liste dans le terminal jusqu'à la
    /// prochaine ligne de commande.
    fallback: bool,
    /// Ce qui devrait être affiché, et ce que la fenêtre a reçu.
    wanted: Option<View>,
    sent: Option<View>,
}

impl Overlay {
    /// Lance la fenêtre si elle est disponible. `unavailable` reçoit un
    /// signal quand elle ne trouve pas le curseur.
    pub fn spawn(unavailable: Sender<()>) -> Option<Self> {
        let enabled = match std::env::var(OVERLAY_ENV).as_deref() {
            Ok("0") => false,
            Ok("1") => true,
            _ => cfg!(windows),
        };
        if !enabled {
            return None;
        }
        let program = program()?;
        let mut command = Command::new(program);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let ready = Arc::new(AtomicBool::new(false));
        let ready_flag = Arc::clone(&ready);
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<Event>(&line) {
                    Ok(Event::Ready) => ready_flag.store(true, Ordering::Relaxed),
                    Ok(Event::Unavailable) => {
                        let _ = unavailable.send(());
                    }
                    Err(_) => {}
                }
            }
            // La fenêtre s'est arrêtée : retour à la liste dans le terminal.
            ready_flag.store(false, Ordering::Relaxed);
            let _ = unavailable.send(());
        });
        Some(Self {
            child,
            stdin,
            ready,
            fallback: false,
            wanted: None,
            sent: None,
        })
    }

    /// La liste doit passer par la fenêtre.
    pub fn usable(&self) -> bool {
        !self.fallback && self.ready.load(Ordering::Relaxed)
    }

    pub fn show(&mut self, view: Option<View>) {
        self.wanted = view;
    }

    pub fn hide(&mut self) {
        self.wanted = None;
    }

    /// Le curseur est introuvable : la fenêtre s'est cachée d'elle-même.
    pub fn fall_back(&mut self) {
        self.fallback = true;
        self.wanted = None;
        self.sent = None;
    }

    /// Nouvelle ligne de commande : on retente la fenêtre.
    pub fn retry(&mut self) {
        self.fallback = false;
    }

    /// Envoie l'état voulu s'il a changé. Appelé une fois par mise à jour,
    /// pour que « cacher puis réafficher la même liste » ne fasse rien.
    pub fn flush(&mut self) {
        if self.wanted == self.sent {
            return;
        }
        let request = match &self.wanted {
            Some(view) => Request::Show(view.clone()),
            None => Request::Hide,
        };
        if let Ok(mut line) = serde_json::to_string(&request) {
            line.push('\n');
            if self
                .stdin
                .write_all(line.as_bytes())
                .and_then(|_| self.stdin.flush())
                .is_err()
            {
                self.ready.store(false, Ordering::Relaxed);
            }
        }
        self.sent = self.wanted.clone();
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `easytab-overlay`, à côté de `easytab-term`.
fn program() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.with_file_name(format!("easytab-overlay{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}
