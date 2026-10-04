//! Position de la fenêtre par rapport au curseur du terminal.

/// Rectangle en pixels de l'écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Curseur lu à l'écran : sa case, la largeur d'une case, la fenêtre du
/// terminal et l'élément qui a le focus (un onglet ou un terminal de VS Code).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caret {
    pub rect: Rect,
    pub cell_width: f64,
    pub window: isize,
    pub element: u64,
}

impl Caret {
    /// Le même curseur, déplacé de `rows` lignes et `cols` colonnes.
    fn shifted(self, rows: isize, cols: isize) -> Self {
        let dx = (cols as f64 * self.cell_width).round() as i32;
        let dy = rows as i32 * self.height();
        Self {
            rect: Rect {
                left: self.rect.left + dx,
                top: self.rect.top + dy,
                right: self.rect.right + dx,
                bottom: self.rect.bottom + dy,
            },
            ..self
        }
    }

    fn height(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }

    /// Même terminal (fenêtre et élément).
    pub fn same_terminal(&self, other: &Caret) -> bool {
        self.window == other.window && self.element == other.element
    }
}

/// Lectures en retard sur la même ligne tolérées avant d'y croire.
const BEHIND_LIMIT: u32 = 3;

/// Suit la position du curseur à l'écran. Les terminaux exposent leur curseur
/// avec un temps de retard (VS Code surtout), et le shell repasse souvent par
/// le début de la ligne pour la redessiner (PowerShell à chaque frappe) : une
/// lecture peut donc montrer le curseur plus à gauche qu'il n'est. On garde
/// une position de référence, avec la ligne et la colonne du curseur dans le
/// terminal à ce moment-là, et on en déduit la position actuelle. Une lecture
/// plus à droite ou sur une autre ligne est retenue tout de suite ; une
/// lecture plus à gauche sur la même ligne seulement si elle se répète.
#[derive(Debug, Default)]
pub struct Tracker {
    anchor: Option<(Caret, usize, usize)>,
    /// Lectures plus à gauche que prévu à la suite.
    behind: u32,
    /// Largeur d'une case mesurée entre deux lectures sur la même ligne
    /// (plus juste que l'estimation d'une seule lecture).
    cell_width: Option<f64>,
}

impl Tracker {
    /// Nouvelle lecture alors que le curseur du terminal est en (`row`, `col`).
    /// Renvoie vrai si la position déduite a changé.
    pub fn read(&mut self, caret: Caret, row: usize, col: usize) -> bool {
        let Some(expected) = self.caret(row, col).filter(|e| e.same_terminal(&caret)) else {
            self.cell_width = None;
            self.behind = 0;
            self.anchor = Some((caret, row, col));
            return true;
        };
        let dx = caret.rect.left - expected.rect.left;
        let same_row = (caret.rect.top - expected.rect.top).abs() <= caret.height() / 2;
        let close = same_row && (dx.abs() as f64) <= expected.cell_width * 1.5;
        if same_row && !close && dx < 0 {
            self.behind += 1;
            if self.behind < BEHIND_LIMIT {
                return false;
            }
        }
        self.behind = 0;
        if let Some((previous, previous_row, previous_col)) = self.anchor {
            let columns = col as f64 - previous_col as f64;
            let width = (caret.rect.left - previous.rect.left) as f64 / columns;
            let height = caret.height() as f64;
            if close
                && previous_row == row
                && columns.abs() >= 2.0
                && width >= height * 0.3
                && width <= height
            {
                self.cell_width = Some(width);
            }
        }
        self.anchor = Some((caret, row, col));
        expected.rect != caret.rect
    }

    /// Position du curseur quand il est en (`row`, `col`) dans le terminal.
    pub fn caret(&self, row: usize, col: usize) -> Option<Caret> {
        let (mut caret, anchor_row, anchor_col) = self.anchor?;
        if let Some(width) = self.cell_width {
            caret.cell_width = width;
        }
        Some(caret.shifted(
            row as isize - anchor_row as isize,
            col as isize - anchor_col as isize,
        ))
    }

    /// Le terminal suivi.
    pub fn terminal(&self) -> Option<Caret> {
        self.anchor.map(|(caret, _, _)| caret)
    }
}

/// Mesures de la page, en pixels CSS (voir `render` dans `popup.html`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct Measure {
    pub width: f64,
    pub height: f64,
    /// Bords du cadre (la fenêtre a une marge pour l'ombre).
    pub box_left: f64,
    pub box_top: f64,
    pub box_bottom: f64,
}

/// Coin haut gauche et taille de la fenêtre, en pixels de l'écran : comme
/// Fig, le bord gauche du cadre au début du mot tapé (au curseur après une
/// espace), sous le curseur ou au-dessus s'il n'y a pas la place en dessous,
/// sans sortir de l'écran.
pub fn place(
    caret: Rect,
    cell_width: f64,
    word_width: usize,
    measure: Measure,
    scale: f64,
    screen: Rect,
) -> (i32, i32, i32, i32) {
    const GAP: f64 = 2.0;
    let width = (measure.width * scale).ceil() as i32;
    let height = (measure.height * scale).ceil() as i32;
    let word_left = caret.left as f64 - word_width as f64 * cell_width;
    let mut x = (word_left - measure.box_left * scale).round() as i32;
    let mut y = (caret.bottom as f64 + GAP * scale - measure.box_top * scale).round() as i32;
    if y + (measure.box_bottom * scale) as i32 > screen.bottom {
        y = (caret.top as f64 - GAP * scale - measure.box_bottom * scale).round() as i32;
    }
    x = x.min(screen.right - width).max(screen.left);
    y = y.min(screen.bottom - height).max(screen.top);
    (x, y, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };
    const MEASURE: Measure = Measure {
        width: 300.0,
        height: 200.0,
        box_left: 8.0,
        box_top: 6.0,
        box_bottom: 182.0,
    };

    fn caret(left: i32, top: i32) -> Rect {
        Rect {
            left,
            top,
            right: left + 9,
            bottom: top + 20,
        }
    }

    #[test]
    fn starts_the_box_at_the_typed_word() {
        // Mot de 2 colonnes de 9 px : le cadre commence 18 px avant le curseur.
        let (x, y, width, height) = place(caret(500, 300), 9.0, 2, MEASURE, 1.0, SCREEN);
        assert_eq!(x + 8, 500 - 18);
        assert_eq!(y + 6, 320 + 2);
        assert_eq!((width, height), (300, 200));
    }

    #[test]
    fn goes_above_near_the_bottom_and_stays_on_screen() {
        let (x, y, _, _) = place(caret(10, 1000), 9.0, 2, MEASURE, 1.0, SCREEN);
        assert_eq!(x, 0);
        assert_eq!(y + 182, 1000 - 2);
    }

    fn read(left: i32, top: i32) -> Caret {
        Caret {
            rect: caret(left, top),
            cell_width: 9.0,
            window: 7,
            element: 1,
        }
    }

    #[test]
    fn follows_the_terminal_cursor() {
        let mut tracker = Tracker::default();
        assert!(tracker.read(read(100, 40), 2, 10));
        // Trois lettres tapées : la position est déduite tout de suite.
        assert_eq!(tracker.caret(2, 13).unwrap().rect.left, 127);
        assert_eq!(tracker.caret(3, 0).unwrap().rect.top, 60);
    }

    #[test]
    fn ignores_readings_left_behind_by_a_redraw() {
        let mut tracker = Tracker::default();
        tracker.read(read(100, 40), 2, 10);
        // Lecture en colonne 0 pendant que le shell redessine : ignorée.
        assert!(!tracker.read(read(10, 40), 2, 13));
        assert_eq!(tracker.caret(2, 13).unwrap().rect.left, 127);
        // Lecture à jour : retenue.
        assert!(!tracker.read(read(127, 40), 2, 13));
        // La même lecture à gauche plusieurs fois : le terminal a bougé.
        for _ in 0..BEHIND_LIMIT {
            tracker.read(read(10, 40), 2, 13);
        }
        assert_eq!(tracker.caret(2, 13).unwrap().rect.left, 10);
    }

    #[test]
    fn corrects_a_first_reading_taken_too_early() {
        let mut tracker = Tracker::default();
        // Première lecture en colonne 0 alors que le curseur est en colonne 2.
        tracker.read(read(10, 40), 2, 2);
        // La suivante, plus à droite, est retenue tout de suite.
        assert!(tracker.read(read(400, 40), 2, 2));
        assert_eq!(tracker.caret(2, 2).unwrap().rect.left, 400);
    }

    #[test]
    fn starts_over_in_another_terminal() {
        let mut tracker = Tracker::default();
        tracker.read(read(100, 40), 2, 10);
        let other = Caret {
            element: 2,
            ..read(10, 40)
        };
        assert!(tracker.read(other, 2, 10));
        assert_eq!(tracker.terminal().unwrap().element, 2);
    }

    #[test]
    fn measures_the_cell_width_between_readings() {
        let mut tracker = Tracker::default();
        tracker.read(read(100, 40), 2, 10);
        // Les cases font en réalité 8 px, pas les 9 px estimés.
        tracker.read(read(124, 40), 2, 13);
        assert_eq!(tracker.caret(2, 23).unwrap().rect.left, 204);
        assert_eq!(tracker.caret(2, 13).unwrap().cell_width, 8.0);
    }

    #[test]
    fn scales_css_pixels() {
        let (x, _, width, height) = place(caret(1000, 300), 18.0, 1, MEASURE, 2.0, SCREEN);
        assert_eq!(x + 16, 1000 - 18);
        assert_eq!((width, height), (600, 400));
    }
}
