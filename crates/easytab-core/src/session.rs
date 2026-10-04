use std::path::{Path, PathBuf};

use crate::osc::{Marker, OscScanner};

/// Où en est le shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Aucun marqueur reçu (intégration shell absente ou commande terminée).
    Unknown,
    /// Le prompt est en cours d'affichage.
    Prompt,
    /// L'utilisateur tape ; la saisie commence à cette position de l'écran.
    Input { row: u16, col: u16 },
    /// Une commande s'exécute.
    Running,
}

/// Copie de l'écran du terminal plus l'état du shell, alimentée par le wrapper PTY.
pub struct Session {
    parser: vt100::Parser,
    scanner: OscScanner,
    phase: Phase,
    last_exit_code: Option<i32>,
    cwd: Option<PathBuf>,
}

impl Session {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: {
                let (rows, cols) = usable_size(rows, cols);
                vt100::Parser::new(rows, cols, 0)
            },
            scanner: OscScanner::new(),
            phase: Phase::Unknown,
            last_exit_code: None,
            cwd: None,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn last_exit_code(&self) -> Option<i32> {
        self.last_exit_code
    }

    /// Dossier courant annoncé par le shell (`OSC 7`).
    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    /// Vrai si le flux est entre deux séquences : on peut alors écrire
    /// par-dessus sans couper une séquence du shell.
    pub fn at_boundary(&self) -> bool {
        self.scanner.is_idle()
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = usable_size(rows, cols);
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// Octets écrits par le shell vers le terminal.
    pub fn feed_output(&mut self, data: &[u8]) {
        let mut start = 0;
        for (i, &byte) in data.iter().enumerate() {
            if let Some(marker) = self.scanner.push(byte) {
                // L'écran doit être à jour au moment du marqueur pour que la
                // position du curseur soit la bonne.
                self.parser.process(&data[start..=i]);
                start = i + 1;
                self.apply(marker);
            }
        }
        self.parser.process(&data[start..]);
    }

    /// Octets tapés par l'utilisateur vers le shell.
    pub fn feed_input(&mut self, data: &[u8]) {
        // Entrée valide la ligne, même si le shell n'émet pas `OSC 133;C`.
        if matches!(self.phase, Phase::Input { .. }) && data.contains(&b'\r') {
            self.phase = Phase::Running;
        }
    }

    /// Texte tapé entre la fin du prompt et le curseur, si l'utilisateur est en
    /// train de saisir une commande.
    pub fn current_input(&self) -> Option<String> {
        let Phase::Input { row, col } = self.phase else {
            return None;
        };
        let screen = self.parser.screen();
        if screen.alternate_screen() {
            return None;
        }
        let (cursor_row, cursor_col) = screen.cursor_position();
        if (cursor_row, cursor_col) < (row, col) {
            return None;
        }
        Some(screen.contents_between(row, col, cursor_row, cursor_col))
    }

    fn apply(&mut self, marker: Marker) {
        self.phase = match marker {
            Marker::WorkingDirectory(path) => {
                self.cwd = Some(path);
                return;
            }
            Marker::PromptStart => Phase::Prompt,
            Marker::InputStart => {
                let (row, col) = self.parser.screen().cursor_position();
                Phase::Input { row, col }
            }
            Marker::CommandStart => Phase::Running,
            Marker::CommandEnd { exit_code } => {
                self.last_exit_code = exit_code;
                Phase::Unknown
            }
        };
    }
}

/// Taille par défaut quand le terminal ne donne pas la sienne (vt100 refuse une
/// grille vide).
fn usable_size(rows: u16, cols: u16) -> (u16, u16) {
    if rows == 0 || cols == 0 {
        (24, 80)
    } else {
        (rows, cols)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROMPT: &[u8] = b"\x1b]133;A\x07\x1b[32m~/projet\x1b[0m $ \x1b]133;B\x07";

    #[test]
    fn tracks_typed_command() {
        let mut session = Session::new(24, 80);
        session.feed_output(PROMPT);
        assert_eq!(session.current_input().as_deref(), Some(""));

        session.feed_output(b"git ch");
        assert_eq!(session.current_input().as_deref(), Some("git ch"));
    }

    #[test]
    fn keeps_trailing_spaces() {
        let mut session = Session::new(24, 80);
        session.feed_output(PROMPT);
        session.feed_output(b"git ");
        assert_eq!(session.current_input().as_deref(), Some("git "));
    }

    #[test]
    fn follows_cursor_moves_and_deletes() {
        let mut session = Session::new(24, 80);
        session.feed_output(PROMPT);
        session.feed_output(b"git checkout");
        // Deux retours arrière tels que les shells les redessinent.
        session.feed_output(b"\x08 \x08\x08 \x08");
        assert_eq!(session.current_input().as_deref(), Some("git checko"));
        // Flèche gauche : seul le texte avant le curseur compte.
        session.feed_output(b"\x1b[4D");
        assert_eq!(session.current_input().as_deref(), Some("git ch"));
    }

    #[test]
    fn handles_multiline_prompt() {
        let mut session = Session::new(24, 80);
        session.feed_output(b"\x1b]133;A\x07~/projet (main)\r\n> \x1b]133;B\x07docker ps");
        assert_eq!(session.current_input().as_deref(), Some("docker ps"));
    }

    #[test]
    fn handles_wrapped_input() {
        let mut session = Session::new(24, 10);
        session.feed_output(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        session.feed_output(b"echo 123456789");
        assert_eq!(session.current_input().as_deref(), Some("echo 123456789"));
    }

    #[test]
    fn enter_ends_input_and_markers_track_the_command() {
        let mut session = Session::new(24, 80);
        session.feed_output(PROMPT);
        session.feed_output(b"false");
        session.feed_input(b"\r");
        assert_eq!(session.phase(), Phase::Running);
        assert_eq!(session.current_input(), None);

        session.feed_output(b"\r\n\x1b]133;C\x07\x1b]133;D;1\x07");
        assert_eq!(session.phase(), Phase::Unknown);
        assert_eq!(session.last_exit_code(), Some(1));
    }

    #[test]
    fn tolerates_unknown_terminal_size() {
        let mut session = Session::new(0, 0);
        session.resize(0, 0);
        session.feed_output(PROMPT);
        session.feed_output(b"git ch");
        assert_eq!(session.current_input().as_deref(), Some("git ch"));
    }

    #[test]
    fn tracks_working_directory() {
        let mut session = Session::new(24, 80);
        session.feed_output(b"\x1b]7;file://pc/tmp/projet\x07");
        assert_eq!(session.cwd(), Some(Path::new("/tmp/projet")));
    }

    #[test]
    fn nothing_without_shell_integration() {
        let mut session = Session::new(24, 80);
        session.feed_output(b"$ ls");
        assert_eq!(session.current_input(), None);
    }

    #[test]
    fn ignores_fullscreen_apps() {
        let mut session = Session::new(24, 80);
        session.feed_output(PROMPT);
        session.feed_output(b"\x1b[?1049h");
        assert_eq!(session.current_input(), None);
    }
}
