//! Position de la fenêtre par rapport au curseur du terminal.

/// Rectangle en pixels de l'écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Curseur lu à l'écran : sa case, la largeur d'une case et la fenêtre du
/// terminal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caret {
    pub rect: Rect,
    pub cell_width: f64,
    pub window: isize,
}

impl Caret {
    /// Le même curseur, déplacé de `rows` lignes et `cols` colonnes.
    fn shifted(self, rows: isize, cols: isize) -> Self {
        let dx = (cols as f64 * self.cell_width).round() as i32;
        let dy = rows as i32 * (self.rect.bottom - self.rect.top);
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
}

/// Suit la position du curseur à l'écran. Les terminaux exposent leur curseur
/// avec un temps de retard (VS Code surtout) : une lecture peut dater d'avant
/// la dernière frappe, voire d'un passage en colonne 0 pendant que le shell
/// redessine la ligne. On garde donc une position de référence, avec la ligne
/// et la colonne du curseur dans le terminal à ce moment-là, et on en déduit
/// la position actuelle. Une lecture qui ne colle pas avec cette déduction
/// n'est retenue que si la suivante donne la même chose.
#[derive(Debug, Default)]
pub struct Tracker {
    anchor: Option<(Caret, usize, usize)>,
    candidate: Option<Caret>,
    /// Largeur d'une case mesurée entre deux lectures sur la même ligne
    /// (plus juste que l'estimation d'une seule lecture).
    cell_width: Option<f64>,
}

impl Tracker {
    /// Nouvelle lecture alors que le curseur du terminal est en (`row`, `col`).
    /// Renvoie vrai si la position de référence a changé.
    pub fn read(&mut self, caret: Caret, row: usize, col: usize) -> bool {
        let expected = self.caret(row, col);
        let agrees = |other: Option<Caret>, slack: f64| {
            other.is_some_and(|other| {
                other.window == caret.window
                    && ((other.rect.left - caret.rect.left).abs() as f64) <= slack
                    && (other.rect.top - caret.rect.top).abs()
                        <= (caret.rect.bottom - caret.rect.top) / 2
            })
        };
        if agrees(expected, caret.cell_width * 1.5) || agrees(self.candidate, 2.0) {
            let changed = expected.map(|e| e.rect) != Some(caret.rect);
            if let Some((previous, previous_row, previous_col)) = self.anchor {
                let columns = col as f64 - previous_col as f64;
                let height = (caret.rect.bottom - caret.rect.top) as f64;
                let width = (caret.rect.left - previous.rect.left) as f64 / columns;
                if previous.window == caret.window
                    && previous_row == row
                    && columns.abs() >= 2.0
                    && width >= height * 0.3
                    && width <= height
                {
                    self.cell_width = Some(width);
                }
            }
            self.anchor = Some((caret, row, col));
            self.candidate = None;
            changed
        } else {
            self.candidate = Some(caret);
            false
        }
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

    pub fn window(&self) -> Option<isize> {
        self.anchor.map(|(caret, _, _)| caret.window)
    }
}

/// Mesures de la page, en pixels CSS (voir `render` dans `popup.html`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct Measure {
    pub width: f64,
    pub height: f64,
    /// Début des noms dans la fenêtre.
    pub label_x: f64,
    /// Haut et bas du cadre (la fenêtre a une marge pour l'ombre).
    pub box_top: f64,
    pub box_bottom: f64,
}

/// Coin haut gauche et taille de la fenêtre, en pixels de l'écran : sous le
/// curseur, les noms alignés sous le mot tapé, au-dessus s'il n'y a pas la
/// place en dessous, sans sortir de l'écran.
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
    let mut x = (word_left - measure.label_x * scale).round() as i32;
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
        label_x: 40.0,
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
    fn places_names_under_the_typed_word() {
        // Mot de 2 colonnes de 9 px : les noms commencent 18 px avant le curseur.
        let (x, y, width, height) = place(caret(500, 300), 9.0, 2, MEASURE, 1.0, SCREEN);
        assert_eq!(x + 40, 500 - 18);
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
        }
    }

    #[test]
    fn needs_two_matching_readings_to_start() {
        let mut tracker = Tracker::default();
        assert!(!tracker.read(read(100, 40), 2, 10));
        assert!(tracker.caret(2, 10).is_none());
        assert!(tracker.read(read(100, 40), 2, 10));
        assert_eq!(tracker.caret(2, 10).unwrap().rect.left, 100);
    }

    #[test]
    fn follows_the_terminal_cursor_and_ignores_stale_readings() {
        let mut tracker = Tracker::default();
        tracker.read(read(100, 40), 2, 10);
        tracker.read(read(100, 40), 2, 10);
        // Trois lettres tapées : la position est déduite tout de suite.
        assert_eq!(tracker.caret(2, 13).unwrap().rect.left, 127);
        // Lecture en colonne 0 pendant que le shell redessine : ignorée.
        tracker.read(read(10, 40), 2, 13);
        assert_eq!(tracker.caret(2, 13).unwrap().rect.left, 127);
        // Lecture à jour : retenue.
        tracker.read(read(127, 40), 2, 13);
        assert_eq!(tracker.caret(3, 0).unwrap().rect.top, 60);
        // Le terminal a vraiment bougé : deux lectures identiques suffisent.
        tracker.read(read(400, 300), 2, 13);
        tracker.read(read(400, 300), 2, 13);
        assert_eq!(tracker.caret(2, 13).unwrap().rect.left, 400);
    }

    #[test]
    fn measures_the_cell_width_between_readings() {
        let mut tracker = Tracker::default();
        tracker.read(read(100, 40), 2, 10);
        tracker.read(read(100, 40), 2, 10);
        // Les cases font en réalité 8 px, pas les 9 px estimés.
        tracker.read(read(124, 40), 2, 13);
        assert_eq!(tracker.caret(2, 23).unwrap().rect.left, 204);
        assert_eq!(tracker.caret(2, 13).unwrap().cell_width, 8.0);
    }

    #[test]
    fn scales_css_pixels() {
        let (x, _, width, height) = place(caret(1000, 300), 18.0, 1, MEASURE, 2.0, SCREEN);
        assert_eq!(x + 80, 1000 - 18);
        assert_eq!((width, height), (600, 400));
    }
}
