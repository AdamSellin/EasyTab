//! Liste de suggestions dessinée dans le terminal, sous (ou au-dessus de) la
//! ligne en cours. Les cases recouvertes sont restaurées à partir de la copie de
//! l'écran tenue par [`Session`].

use std::io::Write;
use std::path::Path;

use easytab_core::overlay::{Row, View};
use easytab_core::{Completer, Completion, Kind, Session};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Nombre de lignes visibles.
const MAX_ROWS: usize = 8;
/// Largeur maximale de la liste.
const MAX_WIDTH: usize = 72;
/// Largeur maximale de la colonne des noms.
const MAX_LABEL_WIDTH: usize = 48;

/// Largeur minimale du cadre.
const MIN_WIDTH: usize = 24;

const STYLE: &str = "\x1b[0;38;5;252;48;5;236m";
const STYLE_SELECTED: &str = "\x1b[0;38;5;231;48;5;25m";
const STYLE_MATCH: &str = "\x1b[1;38;5;231m";
const STYLE_HINT: &str = "\x1b[22;38;5;244m";
const STYLE_HINT_SELECTED: &str = "\x1b[22;38;5;153m";
const STYLE_BORDER: &str = "\x1b[0;38;5;240;48;5;236m";
const STYLE_FOOTER: &str = "\x1b[0;3;38;5;250;48;5;236m";

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
            _ => Self::parse_win32(data),
        }
    }

    /// Vrai si `data` valide la ligne (Entrée), sous l'une ou l'autre forme.
    pub fn submits(data: &[u8]) -> bool {
        data.contains(&b'\r')
            || win32_records(data)
                .is_some_and(|records| records.iter().any(|r| r.down && r.vk == VK_RETURN))
    }

    /// Touche envoyée en « win32-input-mode » : la pseudo-console Windows le
    /// demande au terminal (`ESC[?9001h`), qui envoie alors chaque touche sous
    /// la forme `ESC[Vk;Sc;Uc;Kd;Cs;Rc_` (appui puis relâchement).
    fn parse_win32(data: &[u8]) -> Option<Key> {
        let records = win32_records(data)?;
        // Séquence VT que la console n'a pas reconnue (`ESC[B` reçu d'un
        // terminal qui n'est pas en win32-input-mode) : elle la transmet
        // caractère par caractère, sans touche virtuelle. On la recompose.
        if records.iter().filter(|r| r.down).all(|r| r.vk == 0) {
            let text: Option<Vec<u8>> = records
                .iter()
                .filter(|r| r.down)
                .map(|r| u8::try_from(r.unicode).ok())
                .collect();
            let text = text?;
            return match text.as_slice() {
                b"\x1b[A" | b"\x1bOA" => Some(Key::Up),
                b"\x1b[B" | b"\x1bOB" => Some(Key::Down),
                b"\x1b" => Some(Key::Dismiss),
                _ => None,
            };
        }
        let mut pressed = records.iter().filter(|r| r.down);
        let record = pressed.next()?;
        if pressed.next().is_some() || record.modifiers & MODIFIERS != 0 {
            return None;
        }
        match record.vk {
            VK_UP => Some(Key::Up),
            VK_DOWN => Some(Key::Down),
            VK_TAB => Some(Key::Accept),
            VK_RETURN => Some(Key::Enter),
            VK_ESCAPE => Some(Key::Dismiss),
            _ => None,
        }
    }
}

/// Retire de `data` les signaux de focus du terminal (`ESC[I` : le terminal
/// prend le focus, `ESC[O` : il le perd) et renvoie le dernier reçu.
pub fn take_focus_events(data: &mut Vec<u8>) -> Option<bool> {
    let mut focused = None;
    let mut i = 0;
    while i + 3 <= data.len() {
        match &data[i..i + 3] {
            b"\x1b[I" | b"\x1b[O" => {
                focused = Some(data[i + 2] == b'I');
                data.drain(i..i + 3);
            }
            _ => i += 1,
        }
    }
    focused
}

/// Réponse du terminal à `ESC[6n` (`ESC[ligne;colonneR`, à partir de 1) :
/// position et emplacement de la séquence dans `data`.
pub fn cursor_report(data: &[u8]) -> Option<(u16, u16, std::ops::Range<usize>)> {
    let mut start = 0;
    while let Some(offset) = data[start..].windows(2).position(|w| w == b"\x1b[") {
        let begin = start + offset;
        let body = &data[begin + 2..];
        if let Some(end) = body
            .iter()
            .position(|&b| !(b.is_ascii_digit() || b == b';'))
        {
            if body[end] == b'R' {
                let text = std::str::from_utf8(&body[..end]).ok()?;
                if let Some((row, col)) = text.split_once(';') {
                    if let (Ok(row), Ok(col)) = (row.parse(), col.parse()) {
                        return Some((row, col, begin..begin + 2 + end + 1));
                    }
                }
            }
        }
        start = begin + 2;
    }
    None
}

const VK_TAB: u32 = 9;
const VK_RETURN: u32 = 13;
const VK_ESCAPE: u32 = 27;
const VK_UP: u32 = 38;
const VK_DOWN: u32 = 40;
/// Maj, Ctrl et Alt dans le champ `Cs` (états des touches de contrôle).
const MODIFIERS: u32 = 0x1f;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Win32Record {
    vk: u32,
    unicode: u32,
    down: bool,
    modifiers: u32,
}

/// Découpe `data` en enregistrements win32-input-mode, si elle n'est faite que
/// de ça.
fn win32_records(data: &[u8]) -> Option<Vec<Win32Record>> {
    let mut records = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let body = rest.strip_prefix(b"\x1b[")?;
        let end = body.iter().position(|&b| b == b'_')?;
        let fields = std::str::from_utf8(&body[..end]).ok()?;
        if !fields.bytes().all(|b| b.is_ascii_digit() || b == b';') {
            return None;
        }
        // Champs absents : valeurs par défaut (0, sauf Rc = 1).
        let mut values = fields.split(';').map(|f| f.parse::<u32>().unwrap_or(0));
        let vk = values.next().unwrap_or(0);
        let _scan_code = values.next();
        let unicode = values.next().unwrap_or(0);
        let down = values.next().unwrap_or(0) == 1;
        let modifiers = values.next().unwrap_or(0);
        records.push(Win32Record {
            vk,
            unicode,
            down,
            modifiers,
        });
        rest = &body[end + 1..];
    }
    (!records.is_empty()).then_some(records)
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
    /// La liste est affichée dans la fenêtre flottante.
    in_overlay: bool,
}

impl Popup {
    /// La liste est à l'écran : les touches de navigation lui reviennent.
    pub fn is_shown(&self) -> bool {
        self.drawn.is_some() || self.in_overlay
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

    /// Contenu de la fenêtre flottante, ou `None` s'il n'y a rien à afficher.
    pub fn view(&mut self, screen: &vt100::Screen) -> Option<View> {
        let Some(completion) = &self.completion else {
            self.in_overlay = false;
            return None;
        };
        let items = &completion.suggestions;
        let visible = items.len().min(MAX_ROWS);
        self.scroll = scrolled(self.selected, self.scroll, visible);
        let query = typed_part(&completion.replace);
        let rows = items
            .iter()
            .skip(self.scroll)
            .take(visible)
            .map(|item| Row {
                label: item.label.clone(),
                hint: item.hint.clone(),
                kind: kind_name(item.kind).to_string(),
                icon: item.icon.clone(),
                matched: matched_chars(&item.label, query),
            })
            .collect();
        self.in_overlay = true;
        Some(View {
            rows,
            selected: self.selected - self.scroll,
            description: items.get(self.selected).and_then(|s| s.description.clone()),
            word_width: completion.replace.width(),
            cursor_row: screen.cursor_position().0 as usize,
            cursor_col: screen.cursor_position().1 as usize,
            total: items.len(),
            first: self.scroll,
        })
    }

    /// La fenêtre flottante est cachée.
    pub fn leave_overlay(&mut self) {
        self.in_overlay = false;
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

    /// Dessine la liste près du curseur, s'il y a des suggestions et la place :
    /// un cadre arrondi, une pastille colorée par type, la partie tapée en
    /// gras, les arguments attendus en gris et, en bas, la description de la
    /// suggestion choisie.
    pub fn draw(&mut self, screen: &vt100::Screen, out: &mut Vec<u8>) {
        let Some(completion) = &self.completion else {
            return;
        };
        let items = &completion.suggestions;
        let (screen_rows, cols) = screen.size();
        let (cursor_row, cursor_col) = screen.cursor_position();
        let visible = items.len().min(MAX_ROWS);
        let footer = items.iter().any(|s| s.description.is_some());
        let height = (visible + 2 + if footer { 2 } else { 0 }) as u16;

        let top = if cursor_row + 1 + height <= screen_rows {
            cursor_row + 1
        } else if cursor_row >= height {
            cursor_row - height
        } else {
            return;
        };

        self.scroll = scrolled(self.selected, self.scroll, visible);

        // « │ ▣ label hint │ » : bordure, espace, pastille (3), espace, texte,
        // espace, bordure.
        const CHROME: usize = 8;
        let text_width = items
            .iter()
            .map(|s| s.label.width() + s.hint.as_ref().map_or(0, |h| 1 + h.width()))
            .max()
            .unwrap_or(0)
            .min(MAX_LABEL_WIDTH);
        let description_width = items
            .iter()
            .filter_map(|s| s.description.as_deref())
            .map(|d| d.lines().next().unwrap_or("").width() + 4)
            .max()
            .unwrap_or(0);
        let width = (text_width + CHROME)
            .max(description_width.min(MAX_WIDTH))
            .max(MIN_WIDTH)
            .min(cols as usize);
        if width < CHROME + 1 {
            return;
        }
        let inner = width - 2;
        let text_width = width - CHROME;
        // Les noms s'alignent sous le mot tapé.
        let word_width = completion.replace.width() as u16;
        let left = cursor_col
            .saturating_sub(word_width + 6)
            .min((cols as usize - width) as u16);
        let query = typed_part(&completion.replace);
        let icons = icons();

        out.extend_from_slice(b"\x1b[?25l");
        let mut row = top;
        let mut line = String::new();
        let mut put = |out: &mut Vec<u8>, line: &mut String| {
            goto(out, row, left);
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\x1b[0m");
            line.clear();
            row += 1;
        };

        border(&mut line, '╭', '╮', inner);
        put(out, &mut line);
        for (i, item) in items.iter().enumerate().skip(self.scroll).take(visible) {
            let selected = i == self.selected;
            let base = if selected { STYLE_SELECTED } else { STYLE };
            line.push_str(STYLE_BORDER);
            line.push('│');
            line.push_str(base);
            line.push(' ');
            line.push_str(&badge(item.kind, icons));
            line.push_str(base);
            line.push(' ');

            let label = fit(&item.label, text_width);
            let mut used = label.width();
            push_highlighted(&mut line, &label, query, base);
            if let Some(hint) = &item.hint {
                let hint = fit(hint, text_width.saturating_sub(used + 1));
                if !hint.is_empty() {
                    line.push(' ');
                    line.push_str(if selected {
                        STYLE_HINT_SELECTED
                    } else {
                        STYLE_HINT
                    });
                    line.push_str(&hint);
                    used += 1 + hint.width();
                }
            }
            line.push_str(base);
            line.push_str(&" ".repeat(text_width.saturating_sub(used) + 1));
            line.push_str(STYLE_BORDER);
            line.push('│');
            put(out, &mut line);
        }
        if footer {
            border(&mut line, '├', '┤', inner);
            put(out, &mut line);
            let description = items
                .get(self.selected)
                .and_then(|s| s.description.as_deref())
                .unwrap_or("");
            let description = fit(description, inner - 2);
            line.push_str(STYLE_BORDER);
            line.push('│');
            line.push_str(STYLE_FOOTER);
            line.push(' ');
            line.push_str(&description);
            line.push_str(&" ".repeat(inner - 1 - description.width()));
            line.push_str(STYLE_BORDER);
            line.push('│');
            put(out, &mut line);
        }
        border(&mut line, '╰', '╯', inner);
        put(out, &mut line);

        restore_cursor(screen, out);
        self.drawn = Some(Area { top, rows: height });
    }
}

/// Première ligne visible pour que la suggestion choisie reste à l'écran.
fn scrolled(selected: usize, scroll: usize, visible: usize) -> usize {
    if selected < scroll {
        selected
    } else if selected >= scroll + visible {
        selected + 1 - visible
    } else {
        scroll
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Command => "command",
        Kind::Subcommand => "subcommand",
        Kind::Option => "option",
        Kind::Value => "value",
        Kind::Folder => "folder",
        Kind::File => "file",
        Kind::Dynamic => "dynamic",
    }
}

/// Ligne horizontale du cadre.
fn border(line: &mut String, start: char, end: char, inner: usize) {
    line.push_str(STYLE_BORDER);
    line.push(start);
    line.extend(std::iter::repeat_n('─', inner));
    line.push(end);
}

/// Jeu de pastilles : symboles sur fond coloré, ou emoji avec
/// `EASYTAB_ICONS=emoji`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Icons {
    Badges,
    Emoji,
}

fn icons() -> Icons {
    static ICONS: std::sync::OnceLock<Icons> = std::sync::OnceLock::new();
    *ICONS.get_or_init(|| match std::env::var("EASYTAB_ICONS").as_deref() {
        Ok("emoji") => Icons::Emoji,
        _ => Icons::Badges,
    })
}

/// Pastille de 3 colonnes qui indique le type de suggestion.
fn badge(kind: Kind, icons: Icons) -> String {
    let (symbol, color, emoji) = match kind {
        Kind::Command => ('>', 30, "🚀"),
        Kind::Subcommand => ('$', 98, "📦"),
        Kind::Option => ('-', 71, "🚩"),
        Kind::Value => ('=', 172, "💡"),
        Kind::Folder => ('/', 33, "📁"),
        Kind::File => ('·', 243, "📄"),
        Kind::Dynamic => ('@', 166, "🌿"),
    };
    match icons {
        Icons::Badges => format!("\x1b[0;1;38;5;231;48;5;{color}m {symbol} "),
        Icons::Emoji => format!("{emoji} "),
    }
}

/// Partie du mot tapé comparée aux noms (après le dernier `/` ou `=`).
fn typed_part(word: &str) -> &str {
    word.rsplit(['/', '=']).next().unwrap_or(word)
}

/// Écrit `label` en mettant en gras les lettres tapées.
fn push_highlighted(line: &mut String, label: &str, query: &str, base: &str) {
    let marked = matched_chars(label, query);
    let mut bold = false;
    for (i, c) in label.chars().enumerate() {
        let want = marked.contains(&i);
        if want != bold {
            line.push_str(if want { STYLE_MATCH } else { base });
            bold = want;
        }
        line.push(c);
    }
    if bold {
        line.push_str(base);
    }
}

/// Positions (en caractères) des lettres de `query` dans `label` : le début
/// d'un des noms (« -a, --all »), le passage qui contient le mot, ou les
/// lettres trouvées une à une.
fn matched_chars(label: &str, query: &str) -> Vec<usize> {
    let label: Vec<char> = label.to_lowercase().chars().collect();
    let query: Vec<char> = query.to_lowercase().chars().collect();
    if query.is_empty() || query.len() > label.len() {
        return Vec::new();
    }
    let starts = std::iter::once(0).chain((1..label.len()).filter(|&i| label[i - 1] == ' '));
    for start in starts {
        if label[start..].starts_with(&query) {
            return (start..start + query.len()).collect();
        }
    }
    if let Some(at) = label.windows(query.len()).position(|w| w == query) {
        return (at..at + query.len()).collect();
    }
    let mut found = Vec::new();
    let mut rest = query.iter().filter(|&&c| c != '-').peekable();
    for (i, c) in label.iter().enumerate() {
        if rest.peek() == Some(&c) {
            found.push(i);
            rest.next();
        }
    }
    if rest.peek().is_none() {
        found
    } else {
        Vec::new()
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
    fn takes_focus_events() {
        let mut data = b"a\x1b[Ob\x1b[I".to_vec();
        assert_eq!(take_focus_events(&mut data), Some(true));
        assert_eq!(data, b"ab");
        let mut data = b"\x1b[A".to_vec();
        assert_eq!(take_focus_events(&mut data), None);
        assert_eq!(data, b"\x1b[A");
    }

    #[test]
    fn finds_cursor_reports() {
        assert_eq!(cursor_report(b"\x1b[12;5R"), Some((12, 5, 0..7)));
        assert_eq!(cursor_report(b"ab\x1b[A\x1b[3;40Rc"), Some((3, 40, 5..12)));
        assert_eq!(cursor_report(b"\x1b[1;5A"), None);
        assert_eq!(cursor_report(b"hello"), None);
    }

    #[test]
    fn parses_win32_input_mode_keys() {
        // Appui puis relâchement, collés dans la même lecture.
        assert_eq!(
            Key::parse(b"\x1b[13;28;13;1;0;1_\x1b[13;28;13;0;0;1_"),
            Some(Key::Enter)
        );
        assert_eq!(Key::parse(b"\x1b[9;15;9;1;0;1_"), Some(Key::Accept));
        assert_eq!(Key::parse(b"\x1b[27;1;27;1;0;1_"), Some(Key::Dismiss));
        assert_eq!(Key::parse(b"\x1b[40;80;0;1;256;1_"), Some(Key::Down));
        assert_eq!(Key::parse(b"\x1b[38;72;0;1;256;1_"), Some(Key::Up));
        // Séquence VT transmise caractère par caractère, sans touche virtuelle.
        assert_eq!(
            Key::parse(b"\x1b[0;0;27;1;0;1_\x1b[0;0;91;1;0;1_\x1b[0;0;66;1;0;1_"),
            Some(Key::Down)
        );
        assert_eq!(Key::parse(b"\x1b[0;0;27;1;0;1_"), Some(Key::Dismiss));
        assert_eq!(Key::parse(b"\x1b[0;0;97;1;0;1_"), None);
        // Relâchement seul, Maj+Tab, lettre : pas pour la liste.
        assert_eq!(Key::parse(b"\x1b[13;28;13;0;0;1_"), None);
        assert_eq!(Key::parse(b"\x1b[9;15;9;1;16;1_"), None);
        assert_eq!(Key::parse(b"\x1b[65;30;97;1;0;1_"), None);
        assert_eq!(Key::parse(b"\x1b[1;5A"), None);

        assert!(Key::submits(b"\x1b[13;28;13;1;0;1_"));
        assert!(Key::submits(b"ls\r"));
        assert!(!Key::submits(b"\x1b[13;28;13;0;0;1_"));
        assert!(!Key::submits(b"\x1b[65;30;97;1;0;1_"));
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
        // Cadre à partir de la ligne 2 (sous le prompt), noms alignés sous « ch ».
        assert!(
            text.contains("\x1b[2;1H\x1b[0;38;5;240;48;5;236m╭"),
            "{text}"
        );
        assert!(text.contains('╰'), "{text}");
        // « ch » en gras dans « checkout ».
        assert!(
            text.contains(&format!("{STYLE_MATCH}ch{STYLE_SELECTED}eckout")),
            "{text}"
        );

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
    fn highlights_typed_letters() {
        assert_eq!(matched_chars("checkout", "ch"), vec![0, 1]);
        assert_eq!(matched_chars("-a, --all", "--al"), vec![4, 5, 6, 7]);
        assert_eq!(matched_chars("cherry-pick", "pick"), vec![7, 8, 9, 10]);
        assert_eq!(matched_chars("checkout", "chk"), vec![0, 1, 4]);
        assert!(matched_chars("checkout", "zz").is_empty());
        assert_eq!(typed_part("src/ma"), "ma");
        assert_eq!(typed_part("--cleanup=st"), "st");
    }

    #[test]
    fn fits_text() {
        assert_eq!(fit("checkout", 20), "checkout");
        assert_eq!(fit("checkout", 5), "chec…");
        assert_eq!(fit("ligne 1\nligne 2", 20), "ligne 1");
    }
}
