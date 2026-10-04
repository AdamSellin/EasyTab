//! `easytab-term` : lance le shell dans un pseudo-terminal et relaie tout entre
//! le vrai terminal et ce shell, en gardant une copie de l'écran pour savoir ce
//! que l'utilisateur tape. Il affiche la liste de suggestions par-dessus et
//! intercepte ↑, ↓, Tab et Échap quand elle est visible.

mod popup;

use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::terminal;
use easytab_core::{Completer, Session};
use popup::{Key, Popup};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// Variable posée dans l'environnement du shell lancé, pour que l'intégration
/// shell sache qu'elle tourne déjà sous EasyTab.
const ACTIVE_ENV: &str = "EASYTAB_TERM";
/// Désactive l'intégration shell (évite que le shell relance EasyTab).
const DISABLE_ENV: &str = "EASYTAB_DISABLE";
/// Si elle est définie, chaque changement de la ligne en cours y est journalisé.
const LOG_ENV: &str = "EASYTAB_LOG";

/// État partagé entre les fils clavier, écran et redimensionnement.
struct Shared {
    session: Session,
    popup: Popup,
    completer: Completer,
    /// Dossier utilisé tant que le shell n'a pas annoncé le sien (`OSC 7`).
    fallback_cwd: PathBuf,
}

impl Shared {
    /// Recalcule et redessine la liste si l'écran est dans un état stable.
    fn refresh(&mut self, frame: &mut Vec<u8>) {
        if !self.session.at_boundary() {
            return;
        }
        self.popup
            .update(&self.session, &self.completer, &self.fallback_cwd);
        self.popup.draw(self.session.screen(), frame);
    }
}

#[derive(Parser)]
#[command(version, about = "Lance un shell sous EasyTab")]
struct Args {
    /// Shell à lancer (par défaut : $SHELL)
    #[arg(long)]
    shell: Option<String>,
    /// Arguments transmis au shell
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

fn main() -> Result<()> {
    let code = run(Args::parse())?;
    std::process::exit(code);
}

fn run(args: Args) -> Result<i32> {
    let shell = args
        .shell
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(default_shell);
    if !terminal_supported() {
        return run_direct(&shell, &args.args);
    }
    let (cols, rows) = terminal_size().unwrap_or((80, 24));

    let pair = native_pty_system()
        .openpty(pty_size(rows, cols))
        .context("ouverture du pseudo-terminal")?;
    let mut cmd = CommandBuilder::new(&shell);
    cmd.args(&args.args);
    let fallback_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    cmd.cwd(&fallback_cwd);
    cmd.env(ACTIVE_ENV, "1");
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("lancement de {shell}"))?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let master = pair.master;
    let shared = Arc::new(Mutex::new(Shared {
        session: Session::new(rows, cols),
        popup: Popup::default(),
        completer: Completer::builtin(),
        fallback_cwd,
    }));
    let mut log = open_log();

    let raw_mode = RawMode::enable()?;

    // Clavier -> shell.
    let input_shared = Arc::clone(&shared);
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = stdin.read(&mut buf) {
            let data = &buf[..n];
            let mut frame = Vec::new();
            let mut to_shell = data.to_vec();
            {
                let mut shared = input_shared.lock().unwrap();
                let Shared { session, popup, .. } = &mut *shared;
                match Key::parse(data).filter(|_| popup.is_shown()) {
                    Some(key) => {
                        popup.erase(session.screen(), &mut frame);
                        to_shell.clear();
                        match key {
                            Key::Up => popup.select(-1),
                            Key::Down => popup.select(1),
                            Key::Dismiss => popup.dismiss(),
                            Key::Accept => to_shell = popup.accept().unwrap_or_default(),
                        }
                        popup.draw(session.screen(), &mut frame);
                    }
                    None => {
                        // La commande part : la liste ne doit pas rester à l'écran.
                        if data.contains(&b'\r') {
                            popup.erase(session.screen(), &mut frame);
                        }
                        session.feed_input(data);
                    }
                }
                if !frame.is_empty() {
                    let mut stdout = io::stdout().lock();
                    let _ = stdout.write_all(&frame).and_then(|_| stdout.flush());
                }
            }
            if !to_shell.is_empty()
                && writer
                    .write_all(&to_shell)
                    .and_then(|_| writer.flush())
                    .is_err()
            {
                break;
            }
        }
    });

    // Suit la taille du terminal (fonctionne aussi sous Windows, sans SIGWINCH).
    let resize_shared = Arc::clone(&shared);
    thread::spawn(move || {
        let mut current = (cols, rows);
        loop {
            thread::sleep(Duration::from_millis(200));
            if let Some(size) = terminal_size() {
                if size != current {
                    current = size;
                    let (cols, rows) = size;
                    let _ = master.resize(pty_size(rows, cols));
                    let mut shared = resize_shared.lock().unwrap();
                    shared.popup.forget();
                    shared.session.resize(rows, cols);
                }
            }
        }
    });

    // Shell -> écran.
    let (output_done, output_finished) = mpsc::channel();
    let output_shared = Arc::clone(&shared);
    thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        let mut frame = Vec::new();
        let mut last_input = None;
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            let mut shared = output_shared.lock().unwrap();
            frame.clear();
            // La liste est effacée avant la sortie du shell, pour que l'écran
            // corresponde de nouveau à la copie tenue par la session.
            let Shared { session, popup, .. } = &mut *shared;
            popup.erase(session.screen(), &mut frame);
            frame.extend_from_slice(&buf[..n]);
            session.feed_output(&buf[..n]);
            shared.refresh(&mut frame);

            let mut stdout = io::stdout().lock();
            if stdout
                .write_all(&frame)
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
            if let Some(log) = log.as_mut() {
                let input = shared.session.current_input();
                if input != last_input {
                    let _ = writeln!(log, "{:?} {:?}", shared.session.phase(), input);
                    last_input = input;
                }
            }
        }
        let _ = output_done.send(());
    });

    let status = child.wait().context("attente du shell")?;
    // Laisse la sortie se vider ; sous Windows, ConPTY ne signale pas toujours
    // la fin du flux, d'où le délai maximum.
    let _ = output_finished.recv_timeout(Duration::from_millis(500));
    drop(raw_mode);
    Ok(status.exit_code() as i32)
}

/// EasyTab a besoin d'un vrai terminal des deux côtés.
fn terminal_supported() -> bool {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return false;
    }
    if !stdin_is_console() {
        // mintty (fenêtre « Git Bash » par défaut) ne donne qu'un tuyau aux
        // programmes Windows : la frappe n'arrive qu'à l'appui sur Entrée.
        eprintln!(
            "easytab : ce terminal ne fournit pas de console Windows, les suggestions sont \
             désactivées. Ouvre Git Bash dans Windows Terminal ou VS Code pour les avoir."
        );
        return false;
    }
    true
}

/// Sous Windows, vrai si l'entrée est une console (et pas un tuyau, même
/// présenté comme un terminal par MSYS).
#[cfg(windows)]
fn stdin_is_console() -> bool {
    use windows_sys::Win32::System::Console::{GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE};
    let mut mode = 0;
    // SAFETY: appels Win32 sans pointeur conservé ; `mode` vit pendant l'appel.
    unsafe { GetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), &mut mode) != 0 }
}

#[cfg(not(windows))]
fn stdin_is_console() -> bool {
    true
}

/// Lance le shell sans EasyTab, avec l'intégration désactivée.
fn run_direct(shell: &str, args: &[String]) -> Result<i32> {
    let status = std::process::Command::new(shell)
        .args(args)
        .env(DISABLE_ENV, "1")
        .status()
        .with_context(|| format!("lancement de {shell}"))?;
    Ok(status.code().unwrap_or(1))
}

/// Taille du terminal (colonnes, lignes), ou `None` si elle est inconnue ou nulle.
fn terminal_size() -> Option<(u16, u16)> {
    terminal::size()
        .ok()
        .filter(|&(cols, rows)| cols > 0 && rows > 0)
}

fn pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn default_shell() -> String {
    if cfg!(windows) {
        "powershell.exe".into()
    } else {
        "/bin/sh".into()
    }
}

fn open_log() -> Option<File> {
    let path = std::env::var_os(LOG_ENV)?;
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Remet le terminal en mode normal même en cas d'erreur.
struct RawMode;

impl RawMode {
    fn enable() -> Result<Self> {
        terminal::enable_raw_mode().context("passage du terminal en mode brut")?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}
