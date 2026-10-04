/// Marqueurs de prompt sémantiques (`OSC 133`, format FinalTerm).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// `OSC 133;A` : le shell commence à afficher le prompt.
    PromptStart,
    /// `OSC 133;B` : le prompt est affiché, l'utilisateur peut taper.
    InputStart,
    /// `OSC 133;C` : la commande commence à s'exécuter.
    CommandStart,
    /// `OSC 133;D[;code]` : la commande est terminée.
    CommandEnd { exit_code: Option<i32> },
}

/// Taille maximale du corps d'une séquence OSC conservée en mémoire.
const MAX_OSC_LEN: usize = 128;

#[derive(Debug, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
}

/// Détecte les séquences `OSC 133` dans un flux d'octets, même coupées entre
/// deux lectures.
#[derive(Debug, Default)]
pub struct OscScanner {
    state: State,
    body: Vec<u8>,
}

impl OscScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Avance d'un octet ; renvoie un marqueur quand une séquence se termine sur
    /// cet octet.
    pub fn push(&mut self, byte: u8) -> Option<Marker> {
        match self.state {
            State::Ground => {
                if byte == 0x1b {
                    self.state = State::Escape;
                }
                None
            }
            State::Escape => {
                self.state = match byte {
                    b']' => {
                        self.body.clear();
                        State::Osc
                    }
                    0x1b => State::Escape,
                    _ => State::Ground,
                };
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

    fn finish(&mut self) -> Option<Marker> {
        self.state = State::Ground;
        parse_osc_133(&self.body)
    }
}

fn parse_osc_133(body: &[u8]) -> Option<Marker> {
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
}
