//! `easytab-term` : lance le shell dans un pseudo-terminal et relaie tout entre
//! le vrai terminal et ce shell, en gardant une copie de l'écran pour savoir ce
//! que l'utilisateur tape. Il affiche la liste de suggestions par-dessus et
//! intercepte ↑, ↓, Tab, Entrée (après ↑/↓) et Échap quand elle est visible.

mod overlay;
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
use easytab_core::{Completer, Generators, Session, Usage};
use overlay::Overlay;
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
    /// Fenêtre flottante, si elle est disponible.
    overlay: Option<Overlay>,
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
        if let Some(overlay) = &mut self.overlay {
            if self
                .session
                .current_input()
                .is_none_or(|input| input.trim().is_empty())
            {
                overlay.retry();
            }
        }
        self.show(frame);
    }

    /// Des suggestions dynamiques sont arrivées : redessine la liste.
    fn reload(&mut self, frame: &mut Vec<u8>) {
        if !self.session.at_boundary() {
            return;
        }
        self.hide(frame);
        self.popup.reload();
        self.refresh(frame);
    }

    /// Affiche la liste dans la fenêtre flottante si possible, sinon dans le
    /// terminal.
    fn show(&mut self, frame: &mut Vec<u8>) {
        match self.overlay.as_mut().filter(|overlay| overlay.usable()) {
            Some(overlay) => overlay.show(self.popup.view()),
            None => self.popup.draw(self.session.screen(), frame),
        }
    }

    /// Cache la liste (restaure l'écran du terminal ou cache la fenêtre).
    fn hide(&mut self, frame: &mut Vec<u8>) {
        self.popup.erase(self.session.screen(), frame);
        self.popup.leave_overlay();
        if let Some(overlay) = &mut self.overlay {
            overlay.hide();
        }
    }

    /// Transmet à la fenêtre flottante le résultat de la mise à jour.
    fn flush_overlay(&mut self) {
        if let Some(overlay) = &mut self.overlay {
            overlay.flush();
        }
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
    let (generated, generated_rx) = mpsc::channel();
    let generators = Generators::start(move || {
        let _ = generated.send(());
    });
    let (unavailable, unavailable_rx) = mpsc::channel();
    let shared = Arc::new(Mutex::new(Shared {
        session: Session::new(rows, cols),
        popup: Popup::default(),
        completer: Completer::builtin()
            .with_generators(generators)
            .with_usage(load_usage()),
        overlay: Overlay::spawn(unavailable),
        fallback_cwd,
    }));
    let mut log = open_log();

    let raw_mode = RawMode::enable()?;
    // Le shell démarre là où se trouve le curseur, pas en haut de l'écran : la
    // copie de l'écran doit le savoir, sinon la liste serait dessinée au mauvais
    // endroit. On demande la position au terminal (`ESC[6n`) ; le fil clavier
    // intercepte la réponse et la passe au fil écran, qui attend un court
    // instant avant de relayer le shell. Sous Windows, la pseudo-console pose
    // déjà cette question elle-même.
    let (position_tx, position_rx) = mpsc::channel::<(u16, u16)>();
    let mut position_tx = Some(position_tx);
    let mut position_rx = Some(position_rx);
    if cfg!(windows) {
        position_tx = None;
        position_rx = None;
    } else {
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(b"\x1b[6n").and_then(|_| stdout.flush());
    }

    // Clavier -> shell.
    let input_shared = Arc::clone(&shared);
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = stdin.read(&mut buf) {
            let mut data = buf[..n].to_vec();
            if let Some((row, col, range)) = position_tx
                .as_ref()
                .and_then(|_| popup::cursor_report(&data))
            {
                if let Some(tx) = position_tx.take() {
                    let _ = tx.send((row, col));
                }
                data.drain(range);
                if data.is_empty() {
                    continue;
                }
            }
            let data = &data[..];
            let mut frame = Vec::new();
            let mut to_shell = data.to_vec();
            {
                let mut shared = input_shared.lock().unwrap();
                match Key::parse(data).filter(|&key| shared.popup.handles(key)) {
                    Some(key) => {
                        shared.hide(&mut frame);
                        to_shell.clear();
                        let popup = &mut shared.popup;
                        match key {
                            Key::Up => popup.select(-1),
                            Key::Down => popup.select(1),
                            Key::Dismiss => popup.dismiss(),
                            Key::Accept | Key::Enter => {
                                to_shell = popup.accept().unwrap_or_default()
                            }
                        }
                        shared.show(&mut frame);
                    }
                    None => {
                        // La commande part : la liste ne doit pas rester à l'écran.
                        if Key::submits(data) {
                            if let Some(line) = shared.session.current_input() {
                                shared.completer.record(&line);
                            }
                            shared.hide(&mut frame);
                            shared.session.feed_input(b"\r");
                        }
                    }
                }
                shared.flush_overlay();
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
                    shared.popup.leave_overlay();
                    if let Some(overlay) = &mut shared.overlay {
                        overlay.hide();
                        overlay.flush();
                    }
                    shared.session.resize(rows, cols);
                }
            }
        }
    });

    // Suggestions dynamiques (branches git…) arrivées en arrière-plan.
    let generated_shared = Arc::clone(&shared);
    thread::spawn(move || {
        while generated_rx.recv().is_ok() {
            // Plusieurs generators finissent souvent ensemble : un seul dessin.
            while generated_rx.try_recv().is_ok() {}
            let mut frame = Vec::new();
            // Le verrou est gardé pendant l'écriture, comme dans les autres fils.
            let mut shared = generated_shared.lock().unwrap();
            shared.reload(&mut frame);
            shared.flush_overlay();
            if !frame.is_empty() {
                let mut stdout = io::stdout().lock();
                let _ = stdout.write_all(&frame).and_then(|_| stdout.flush());
            }
        }
    });

    // La fenêtre flottante ne trouve pas le curseur : liste dans le terminal.
    let fallback_shared = Arc::clone(&shared);
    thread::spawn(move || {
        while unavailable_rx.recv().is_ok() {
            let mut frame = Vec::new();
            let mut shared = fallback_shared.lock().unwrap();
            if let Some(overlay) = &mut shared.overlay {
                overlay.fall_back();
            }
            shared.popup.leave_overlay();
            if shared.session.at_boundary() {
                shared.show(&mut frame);
            }
            if !frame.is_empty() {
                let mut stdout = io::stdout().lock();
                let _ = stdout.write_all(&frame).and_then(|_| stdout.flush());
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
        if let Some(position) = position_rx.take() {
            if let Ok((row, col)) = position.recv_timeout(Duration::from_millis(500)) {
                let goto = format!("\x1b[{row};{col}H");
                output_shared
                    .lock()
                    .unwrap()
                    .session
                    .feed_output(goto.as_bytes());
            }
        }
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            let mut shared = output_shared.lock().unwrap();
            frame.clear();
            // La liste est effacée avant la sortie du shell, pour que l'écran
            // corresponde de nouveau à la copie tenue par la session.
            shared.hide(&mut frame);
            frame.extend_from_slice(&buf[..n]);
            shared.session.feed_output(&buf[..n]);
            shared.refresh(&mut frame);
            shared.flush_overlay();

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

/// Historique d'utilisation (`~/.easytab/usage.json`), qui fait remonter les
/// suggestions les plus utilisées.
fn load_usage() -> Usage {
    match dirs::home_dir() {
        Some(home) => Usage::load(&home.join(".easytab").join("usage.json")),
        None => Usage::default(),
    }
}

fn open_log() -> Option<File> {
    let path = std::env::var_os(LOG_ENV)?;
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Remet le terminal en mode normal même en cas d'erreur.
struct RawMode {
    #[cfg(windows)]
    console: vt::Saved,
}

impl RawMode {
    fn enable() -> Result<Self> {
        terminal::enable_raw_mode().context("passage du terminal en mode brut")?;
        Ok(Self {
            #[cfg(windows)]
            console: vt::enable(),
        })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        #[cfg(windows)]
        vt::restore(&self.console);
        let _ = terminal::disable_raw_mode();
    }
}

/// Modes VT de la console Windows. Sans eux, les réponses du terminal (dont
/// celle à `ESC[6n`, que la pseudo-console attend avant d'afficher quoi que ce
/// soit) et les flèches n'arrivent pas sous forme de séquences.
#[cfg(windows)]
mod vt {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE, DISABLE_NEWLINE_AUTO_RETURN,
        ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_HANDLE,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    /// Modes d'origine de l'entrée et de la sortie, à remettre en partant.
    pub struct Saved {
        input: Option<CONSOLE_MODE>,
        output: Option<CONSOLE_MODE>,
    }

    pub fn enable() -> Saved {
        Saved {
            input: add_mode(STD_INPUT_HANDLE, ENABLE_VIRTUAL_TERMINAL_INPUT),
            output: add_mode(
                STD_OUTPUT_HANDLE,
                ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN,
            ),
        }
    }

    pub fn restore(saved: &Saved) {
        for (handle, mode) in [
            (STD_INPUT_HANDLE, saved.input),
            (STD_OUTPUT_HANDLE, saved.output),
        ] {
            if let Some(mode) = mode {
                // SAFETY: appel Win32 sur un handle standard du processus.
                unsafe { SetConsoleMode(GetStdHandle(handle), mode) };
            }
        }
    }

    /// Ajoute `flags` au mode de la console ; renvoie l'ancien mode.
    fn add_mode(handle: STD_HANDLE, flags: CONSOLE_MODE) -> Option<CONSOLE_MODE> {
        let mut mode = 0;
        // SAFETY: appels Win32 sur un handle standard du processus ; `mode`
        // vit pendant l'appel.
        unsafe {
            let handle = GetStdHandle(handle);
            if GetConsoleMode(handle, &mut mode) == 0 {
                return None;
            }
            SetConsoleMode(handle, mode | flags);
        }
        Some(mode)
    }
}
