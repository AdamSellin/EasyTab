use std::path::{Path, PathBuf};

/// Séquences émises par l'intégration shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marker {
    /// `OSC 133;A` : le shell commence à afficher le prompt.
    PromptStart,
    /// `OSC 133;B` : le prompt est affiché, l'utilisateur peut taper.
    InputStart,
    /// `OSC 133;C` : la commande commence à s'exécuter.
    CommandStart,
    /// `OSC 133;D[;code]` : la commande est terminée.
    CommandEnd { exit_code: Option<i32> },
    /// `OSC 7;file://hôte/chemin` : dossier courant du shell.
    WorkingDirectory(PathBuf),
}

/// Taille maximale du corps d'une séquence OSC conservée en mémoire.
const MAX_OSC_LEN: usize = 4096;

#[derive(Debug, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    /// `ESC` suivi d'octets intermédiaires, comme `ESC ( B`.
    EscapeIntermediate,
    Csi,
    Osc,
    OscEscape,
}

/// Suit le flux de sortie du shell : repère les séquences d'intégration et sait
/// si le flux est entre deux séquences (on peut alors y insérer du texte sans
/// casser une séquence ou un caractère UTF-8 en cours).
#[derive(Debug, Default)]
pub struct OscScanner {
    state: State,
    body: Vec<u8>,
    utf8_pending: u8,
}

impl OscScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Vrai si aucun échappement ni caractère UTF-8 n'est en cours.
    pub fn is_idle(&self) -> bool {
        matches!(self.state, State::Ground) && self.utf8_pending == 0
    }

    /// Avance d'un octet ; renvoie un marqueur quand une séquence se termine sur
    /// cet octet.
    pub fn push(&mut self, byte: u8) -> Option<Marker> {
        match self.state {
            State::Ground => {
                if byte == 0x1b {
                    self.utf8_pending = 0;
                    self.state = State::Escape;
                } else {
                    self.track_utf8(byte);
                }
                None
            }
            State::Escape => {
                self.state = match byte {
                    b']' => {
                        self.body.clear();
                        State::Osc
                    }
                    b'[' => State::Csi,
                    0x1b => State::Escape,
                    0x20..=0x2f => State::EscapeIntermediate,
                    _ => State::Ground,
                };
                None
            }
            State::EscapeIntermediate => {
                if !(0x20..=0x2f).contains(&byte) {
                    self.state = State::Ground;
                }
                None
            }
            State::Csi => {
                if (0x40..=0x7e).contains(&byte) {
                    self.state = State::Ground;
                } else if byte == 0x1b {
                    self.state = State::Escape;
                }
                None
            }
            State::Osc => match byte {
                // BEL termine la séquence.
                0x07 => self.finish(),
                0x1b => {
                    self.state = State::OscEscape;
                    None
                }
                _ => {
                    if self.body.len() < MAX_OSC_LEN {
                        self.body.push(byte);
                    }
                    None
                }
            },
            State::OscEscape => {
                if byte == b'\\' {
                    // ST (ESC \) termine la séquence.
                    self.finish()
                } else {
                    self.state = if byte == b']' {
                        self.body.clear();
                        State::Osc
                    } else {
                        State::Ground
                    };
                    None
                }
            }
        }
    }

    fn track_utf8(&mut self, byte: u8) {
        self.utf8_pending = match byte {
            0x80..=0xbf => self.utf8_pending.saturating_sub(1),
            0xc0..=0xdf => 1,
            0xe0..=0xef => 2,
            0xf0..=0xf7 => 3,
            _ => 0,
        };
    }

    fn finish(&mut self) -> Option<Marker> {
        self.state = State::Ground;
        parse_osc(&self.body)
    }
}

fn parse_osc(body: &[u8]) -> Option<Marker> {
    if let Some(url) = body.strip_prefix(b"7;") {
        return parse_file_url(url).map(Marker::WorkingDirectory);
    }
    let rest = body.strip_prefix(b"133;")?;
    let (&kind, params) = rest.split_first()?;
    match kind {
        b'A' => Some(Marker::PromptStart),
        b'B' => Some(Marker::InputStart),
        b'C' => Some(Marker::CommandStart),
        b'D' => {
            let exit_code = params
                .strip_prefix(b";")
                .and_then(|p| std::str::from_utf8(p).ok())
                .and_then(|p| p.split(';').next())
                .and_then(|p| p.parse().ok());
            Some(Marker::CommandEnd { exit_code })
        }
        _ => None,
    }
}

/// `file://hôte/chemin%20encodé` -> `/chemin encodé`.
fn parse_file_url(url: &[u8]) -> Option<PathBuf> {
    let rest = url.strip_prefix(b"file://")?;
    let path = &rest[rest.iter().position(|&b| b == b'/')?..];
    let mut decoded = Vec::with_capacity(path.len());
    let mut i = 0;
    while i < path.len() {
        let hex = path
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (path[i], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                i += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

/// Git Bash annonce ses dossiers à la façon MSYS (`/c/Users/adam`), et
/// PowerShell sous la forme `/C:/Users/adam` ; les programmes Windows les
/// veulent sous la forme `C:/Users/adam`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn msys_to_windows(path: &Path) -> Option<PathBuf> {
    let path = path.to_str()?;
    let rest = path.strip_prefix('/')?;
    let mut chars = rest.chars();
    let drive = chars.next().filter(char::is_ascii_alphabetic)?;
    // PowerShell annonce `/C:/Users/adam`.
    let after = chars.as_str();
    let after = after.strip_prefix(':').unwrap_or(after);
    if !(after.is_empty() || after.starts_with('/')) {
        return None;
    }
    Some(PathBuf::from(format!(
        "{}:/{}",
        drive.to_ascii_uppercase(),
        after.trim_start_matches('/')
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(input: &[u8]) -> Vec<Marker> {
        let mut scanner = OscScanner::new();
        input.iter().filter_map(|&b| scanner.push(b)).collect()
    }

    #[test]
    fn detects_markers_with_bel_and_st() {
        assert_eq!(
            scan(b"\x1b]133;A\x07$ \x1b]133;B\x1b\\ls\r\n\x1b]133;C\x07"),
            [
                Marker::PromptStart,
                Marker::InputStart,
                Marker::CommandStart
            ]
        );
    }

    #[test]
    fn parses_exit_code() {
        assert_eq!(
            scan(b"\x1b]133;D;127\x07\x1b]133;D\x07"),
            [
                Marker::CommandEnd {
                    exit_code: Some(127)
                },
                Marker::CommandEnd { exit_code: None }
            ]
        );
    }

    #[test]
    fn parses_working_directory() {
        assert_eq!(
            scan(b"\x1b]7;file://machine/home/adam/mes%20projets\x07"),
            [Marker::WorkingDirectory(PathBuf::from(
                "/home/adam/mes projets"
            ))]
        );
    }

    #[test]
    fn ignores_other_osc_sequences() {
        assert_eq!(scan(b"\x1b]0;titre\x07\x1b]1337;x\x07\x1b[31mrouge"), []);
    }

    #[test]
    fn survives_split_reads() {
        let mut scanner = OscScanner::new();
        let mut found = Vec::new();
        for chunk in [&b"\x1b]13"[..], b"3;", b"B", b"\x07"] {
            found.extend(chunk.iter().filter_map(|&b| scanner.push(b)));
        }
        assert_eq!(found, [Marker::InputStart]);
    }

    #[test]
    fn knows_when_a_sequence_is_unfinished() {
        let mut scanner = OscScanner::new();
        let mut feed = |bytes: &[u8]| {
            bytes.iter().for_each(|&b| {
                scanner.push(b);
            });
            scanner.is_idle()
        };
        assert!(feed(b"abc"));
        assert!(!feed(b"\x1b[3"));
        assert!(feed(b"1m"));
        assert!(!feed(b"\x1b("));
        assert!(feed(b"B"));
        assert!(!feed(&"é".as_bytes()[..1]));
        assert!(feed(&"é".as_bytes()[1..]));
        assert!(!feed(b"\x1b]0;ti"));
        assert!(feed(b"tre\x07"));
    }

    #[test]
    fn converts_msys_paths() {
        let convert = |p: &str| msys_to_windows(Path::new(p)).map(|p| p.display().to_string());
        assert_eq!(convert("/c/Users/adam").as_deref(), Some("C:/Users/adam"));
        assert_eq!(convert("/d").as_deref(), Some("D:/"));
        assert_eq!(convert("/C:/Users/adam").as_deref(), Some("C:/Users/adam"));
        assert_eq!(convert("/C:").as_deref(), Some("C:/"));
        assert_eq!(convert("/usr/bin"), None);
        assert_eq!(convert("/tmp"), None);
    }
}
