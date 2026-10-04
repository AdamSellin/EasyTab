//! `easytab-term` : lance le shell dans un pseudo-terminal et relaie tout entre
//! le vrai terminal et ce shell, en gardant une copie de l'écran pour savoir ce
//! que l'utilisateur tape. Pour l'instant il ne modifie rien au flux.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::terminal;
use easytab_core::Session;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// Variable posée dans l'environnement du shell lancé, pour que l'intégration
/// shell sache qu'elle tourne déjà sous EasyTab.
const ACTIVE_ENV: &str = "EASYTAB_TERM";
/// Si elle est définie, chaque changement de la ligne en cours y est journalisé.
const LOG_ENV: &str = "EASYTAB_LOG";

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
    let (cols, rows) = terminal_size().unwrap_or((80, 24));

    let pair = native_pty_system()
        .openpty(pty_size(rows, cols))
        .context("ouverture du pseudo-terminal")?;
    let mut cmd = CommandBuilder::new(&shell);
    cmd.args(&args.args);
    if let Ok(dir) = std::env::current_dir() {
        cmd.cwd(dir);
    }
    cmd.env(ACTIVE_ENV, "1");
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("lancement de {shell}"))?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let master = pair.master;
    let session = Arc::new(Mutex::new(Session::new(rows, cols)));
    let mut log = open_log();

    let raw_mode = RawMode::enable()?;

    // Clavier -> shell.
    let input_session = Arc::clone(&session);
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = stdin.read(&mut buf) {
            input_session.lock().unwrap().feed_input(&buf[..n]);
            if writer
                .write_all(&buf[..n])
                .and_then(|_| writer.flush())
                .is_err()
            {
                break;
            }
        }
    });

    // Suit la taille du terminal (fonctionne aussi sous Windows, sans SIGWINCH).
    let resize_session = Arc::clone(&session);
    thread::spawn(move || {
        let mut current = (cols, rows);
        loop {
            thread::sleep(Duration::from_millis(200));
            if let Some(size) = terminal_size() {
                if size != current {
                    current = size;
                    let (cols, rows) = size;
                    let _ = master.resize(pty_size(rows, cols));
                    resize_session.lock().unwrap().resize(rows, cols);
                }
            }
        }
    });

    // Shell -> écran.
    let (output_done, output_finished) = mpsc::channel();
    let output_session = Arc::clone(&session);
    thread::spawn(move || {
        let mut stdout = io::stdout().lock();
        let mut buf = [0u8; 16 * 1024];
        let mut last_input = None;
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            if stdout
                .write_all(&buf[..n])
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
            let mut session = output_session.lock().unwrap();
            session.feed_output(&buf[..n]);
            if let Some(log) = log.as_mut() {
                let input = session.current_input();
                if input != last_input {
                    let _ = writeln!(log, "{:?} {:?}", session.phase(), input);
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
