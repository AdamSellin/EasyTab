//! Liste de suggestions dessinée dans le terminal, sous (ou au-dessus de) la
//! ligne en cours. Les cases recouvertes sont restaurées à partir de la copie de
//! l'écran tenue par [`Session`].

use std::io::Write;
use std::path::Path;

use easytab_core::{Completer, Completion, Kind, Session};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Nombre de lignes visibles.
const MAX_ROWS: usize = 8;
/// Largeur maximale de la liste.
const MAX_WIDTH: usize = 72;
/// Largeur maximale de la colonne des noms.
const MAX_LABEL_WIDTH: usize = 36;

const STYLE: &str = "\x1b[0;38;5;252;48;5;237m";
const STYLE_SELECTED: &str = "\x1b[0;1;38;5;231;48;5;25m";
const STYLE_DESCRIPTION: &str = "\x1b[22;38;5;246m";
const STYLE_DESCRIPTION_SELECTED: &str = "\x1b[22;38;5;153m";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Accept,
    /// Entrée : insère la suggestion seulement si l'utilisateur en a choisi une
    /// avec ↑/↓ ; sinon la commande part normalement.
    Enter,
    Dismiss,
}

impl Key {
    /// Touches gérées par la liste quand elle est affichée.
    pub fn parse(data: &[u8]) -> Option<Key> {
        match data {
            b"\x1b[A" | b"\x1bOA" => Some(Key::Up),
            b"\x1b[B" | b"\x1bOB" => Some(Key::Down),
            b"\t" => Some(Key::Accept),
            b"\r" => Some(Key::Enter),
            b"\x1b" => Some(Key::Dismiss),
            _ => None,
        }
    }
}

/// Lignes de l'écran recouvertes par la liste.
#[derive(Debug, Clone, Copy)]
struct Area {
    top: u16,
    rows: u16,
}

#[derive(Default)]
pub struct Popup {
    completion: Option<Completion>,
    selected: usize,
    scroll: usize,
    drawn: Option<Area>,
    last_input: Option<String>,
    /// Ligne pour laquelle l'utilisateur a fermé la liste avec Échap.
    dismissed_for: Option<String>,
    /// De nouvelles suggestions dynamiques sont arrivées : recalculer même si la
    /// ligne n'a pas changé.
    stale: bool,
    /// L'utilisateur a déplacé la sélection avec ↑/↓ depuis la dernière frappe.
    navigated: bool,
}

impl Popup {
    /// La liste est à l'écran : les touches de navigation lui reviennent.
    pub fn is_shown(&self) -> bool {
        self.drawn.is_some()
    }

    /// Recalcule les suggestions si la ligne en cours a changé.
    pub fn update(&mut self, session: &Session, completer: &Completer, fallback_cwd: &Path) {
        let input = session.current_input();
        let stale = std::mem::take(&mut self.stale);
        if input == self.last_input && !stale {
            return;
        }
        // Mêmes mots, nouveaux résultats : la sélection reste sur le même nom.
        let keep = self
            .completion
            .take()
            .filter(|_| input == self.last_input)
            .and_then(|c| c.suggestions.into_iter().nth(self.selected))
            .map(|s| s.label);
        if input != self.last_input {
            self.navigated = false;
        }
        self.last_input = input.clone();
        self.selected = 0;
        self.scroll = 0;

        let Some(input) = input else {
            return;
        };
        if self.dismissed_for.as_ref() == Some(&input) {
            return;
        }
        self.dismissed_for = None;
        if input.trim().is_empty() || input.contains('\n') {
            return;
        }
        let completion = completer.complete(&input, session.cwd().unwrap_or(fallback_cwd));
        let only_exact = matches!(
            completion.suggestions.as_slice(),
            [only] if only.insert == completion.replace
        );
        if !completion.suggestions.is_empty() && !only_exact {
            if let Some(label) = keep {
                self.selected = completion
                    .suggestions
                    .iter()
                    .position(|s| s.label == label)
                    .unwrap_or(0);
            }
            self.completion = Some(completion);
        }
    }

    /// Les suggestions dynamiques ont changé : la prochaine mise à jour
    /// recalcule la liste.
    pub fn reload(&mut self) {
        self.stale = true;
    }

    /// Oublie la liste sans restaurer l'écran (après un redimensionnement, le
    /// shell redessine tout).
    pub fn forget(&mut self) {
        self.completion = None;
        self.drawn = None;
        self.last_input = None;
    }

    pub fn select(&mut self, delta: isize) {
        if let Some(completion) = &self.completion {
            let len = completion.suggestions.len() as isize;
            self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
            self.navigated = true;
        }
    }

    /// Vrai si la touche revient à la liste plutôt qu'au shell.
    pub fn handles(&self, key: Key) -> bool {
        self.is_shown() && (key != Key::Enter || self.navigated)
    }

    pub fn dismiss(&mut self) {
        self.dismissed_for = self.last_input.clone();
        self.completion = None;
    }

    /// Octets à envoyer au shell pour insérer la suggestion choisie.
    pub fn accept(&mut self) -> Option<Vec<u8>> {
        let completion = self.completion.take()?;
        let suggestion = completion.suggestions.get(self.selected)?;
        let mut insert = suggestion.insert.clone();
        if suggestion.append_space {
            insert.push(' ');
        }
        // La ligne change : la prochaine mise à jour recalculera la liste.
        self.last_input = None;
        Some(match insert.strip_prefix(&completion.replace) {
            Some(rest) => rest.as_bytes().to_vec(),
            None => {
                // Efface le mot tapé (DEL) puis écrit la suggestion.
                let mut bytes = vec![0x7f; completion.replace.chars().count()];
                bytes.extend_from_slice(insert.as_bytes());
                bytes
            }
        })
    }

    /// Restaure les lignes recouvertes par la liste.
    pub fn erase(&mut self, screen: &vt100::Screen, out: &mut Vec<u8>) {
        let Some(area) = self.drawn.take() else {
            return;
        };
        let (_, cols) = screen.size();
        let rows: Vec<Vec<u8>> = screen
            .rows_formatted(0, cols)
            .skip(area.top as usize)
            .take(area.rows as usize)
            .collect();
        for (i, row) in rows.iter().enumerate() {
            goto(out, area.top + i as u16, 0);
            out.extend_from_slice(b"\x1b[0m\x1b[2K");
            out.extend_from_slice(row);
        }
        restore_cursor(screen, out);
    }

    /// Dessine la liste près du curseur, s'il y a des suggestions et la place.
    pub fn draw(&mut self, screen: &vt100::Screen, out: &mut Vec<u8>) {
        let Some(completion) = &self.completion else {
            return;
        };
        let items = &completion.suggestions;
        let (screen_rows, cols) = screen.size();
        let (cursor_row, cursor_col) = screen.cursor_position();
        let height = items.len().min(MAX_ROWS) as u16;

        let top = if cursor_row + 1 + height <= screen_rows {
            cursor_row + 1
        } else if cursor_row >= height {
            cursor_row - height
        } else {
            return;
        };

        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height as usize {
            self.scroll = self.selected + 1 - height as usize;
        }

        let label_width = items
            .iter()
            .map(|s| s.label.width())
            .max()
            .unwrap_or(0)
            .min(MAX_LABEL_WIDTH);
        let description_width = items
            .iter()
            .filter_map(|s| s.description.as_deref())
            .map(|d| d.width())
            .max()
            .unwrap_or(0)
            .min(MAX_WIDTH.saturating_sub(label_width + 6));
        // " i label  description "
        let mut width = 3 + label_width + 1;
        if description_width > 0 {
            width += 2 + description_width;
        }
        let width = width.min(cols as usize);
        let word_width = completion.replace.width() as u16;
        let left = cursor_col
            .saturating_sub(word_width + 3)
            .min((cols as usize - width) as u16);

        out.extend_from_slice(b"\x1b[?25l");
        for (i, item) in items
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(height as usize)
        {
            let selected = i == self.selected;
            goto(out, top + (i - self.scroll) as u16, left);
            let mut line = String::new();
            line.push_str(if selected { STYLE_SELECTED } else { STYLE });
            line.push(' ');
            line.push(icon(item.kind));
            line.push(' ');
            let label = fit(&item.label, label_width);
            let mut used = 3 + label.width();
            line.push_str(&label);
            if description_width > 0 {
                let available = width.saturating_sub(used + 3);
                let description = fit(item.description.as_deref().unwrap_or(""), available);
                if !description.is_empty() {
                    let pad = label_width.saturating_sub(label.width()) + 2;
                    if used + pad + description.width() < width {
                        line.push_str(&" ".repeat(pad));
                        line.push_str(if selected {
                            STYLE_DESCRIPTION_SELECTED
                        } else {
                            STYLE_DESCRIPTION
                        });
                        line.push_str(&description);
                        used += pad + description.width();
                    }
                }
            }
            line.push_str(&" ".repeat(width.saturating_sub(used)));
            line.push_str("\x1b[0m");
            out.extend_from_slice(line.as_bytes());
        }
        restore_cursor(screen, out);
        self.drawn = Some(Area { top, rows: height });
    }
}

fn icon(kind: Kind) -> char {
    match kind {
        Kind::Command => '$',
        Kind::Subcommand => '›',
        Kind::Option => '-',
        Kind::Value => '=',
        Kind::Folder => '/',
        Kind::File => '·',
    }
}

/// Coupe `text` à `width` colonnes, avec « … » s'il dépasse.
fn fit(text: &str, width: usize) -> String {
    let text = text.lines().next().unwrap_or("");
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

fn goto(out: &mut Vec<u8>, row: u16, col: u16) {
    let _ = write!(out, "\x1b[{};{}H", row + 1, col + 1);
}

/// Remet le curseur et les attributs du shell tels qu'ils étaient.
fn restore_cursor(screen: &vt100::Screen, out: &mut Vec<u8>) {
    let (row, col) = screen.cursor_position();
    goto(out, row, col);
    out.extend_from_slice(b"\x1b[0m");
    out.extend_from_slice(&screen.attributes_formatted());
    if !screen.hide_cursor() {
        out.extend_from_slice(b"\x1b[?25h");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROMPT: &[u8] = b"\x1b]133;A\x07$ \x1b]133;B\x07";

    fn session_with(input: &[u8]) -> Session {
        let mut session = Session::new(24, 80);
        session.feed_output(PROMPT);
        session.feed_output(input);
        session
    }

    fn popup_for(session: &Session) -> Popup {
        let mut popup = Popup::default();
        popup.update(session, &Completer::builtin(), Path::new("/"));
        popup
    }

    #[test]
    fn parses_keys() {
        assert_eq!(Key::parse(b"\x1b[A"), Some(Key::Up));
        assert_eq!(Key::parse(b"\x1bOB"), Some(Key::Down));
        assert_eq!(Key::parse(b"\t"), Some(Key::Accept));
        assert_eq!(Key::parse(b"\x1b"), Some(Key::Dismiss));
        assert_eq!(Key::parse(b"\r"), Some(Key::Enter));
        assert_eq!(Key::parse(b"a"), None);
    }

    #[test]
    fn enter_inserts_only_after_choosing_with_the_arrows() {
        let session = session_with(b"git ch");
        let mut popup = popup_for(&session);
        let mut out = Vec::new();
        popup.draw(session.screen(), &mut out);
        assert!(popup.handles(Key::Down));
        assert!(!popup.handles(Key::Enter));
        popup.select(1);
        assert!(popup.handles(Key::Enter));
    }

    #[test]
    fn accept_sends_the_missing_part() {
        let session = session_with(b"git chec");
        let mut popup = popup_for(&session);
        assert_eq!(popup.accept().as_deref(), Some(&b"kout "[..]));
    }

    #[test]
    fn selection_wraps_around() {
        let session = session_with(b"git ch");
        let mut popup = popup_for(&session);
        popup.select(-1);
        let bytes = popup.accept().unwrap();
        // Dernière suggestion commençant par « ch ».
        assert!(!bytes.is_empty());
        assert_ne!(bytes, b"eckout ");
    }

    #[test]
    fn draws_then_erases_below_the_cursor() {
        let session = session_with(b"git ch");
        let mut popup = popup_for(&session);
        let mut out = Vec::new();
        popup.draw(session.screen(), &mut out);
        assert!(popup.is_shown());
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("checkout"), "{text}");
        // Ligne 2 de l'écran (sous le prompt), colonne du mot « ch » moins l'icône.
        assert!(text.contains("\x1b[2;4H"), "{text}");

        out.clear();
        popup.erase(session.screen(), &mut out);
        assert!(!popup.is_shown());
        assert!(String::from_utf8_lossy(&out).contains("\x1b[2;1H"));
    }

    #[test]
    fn escape_hides_until_the_line_changes() {
        let mut session = session_with(b"git ch");
        let completer = Completer::builtin();
        let mut popup = Popup::default();
        popup.update(&session, &completer, Path::new("/"));
        popup.dismiss();
        popup.update(&session, &completer, Path::new("/"));
        assert!(popup.accept().is_none());

        session.feed_output(b"e");
        popup.update(&session, &completer, Path::new("/"));
        assert!(popup.accept().is_some());
    }

    #[test]
    fn fits_text() {
        assert_eq!(fit("checkout", 20), "checkout");
        assert_eq!(fit("checkout", 5), "chec…");
        assert_eq!(fit("ligne 1\nligne 2", 20), "ligne 1");
    }
}
