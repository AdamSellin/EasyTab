//! Position du curseur estimée à partir de la fenêtre du terminal, quand le
//! système ne la donne pas (macOS sans accès à l'accessibilité, Linux) : la
//! grille de `cols` × `rows` cases remplit la fenêtre, sous sa barre de titre.

use crate::placement::Rect;
use crate::protocol::View;

/// Plus grande case de texte crédible, en pixels.
const MAX_CELL: f64 = 120.0;
/// Rapport hauteur / largeur maximal d'une case : au-delà, la fenêtre a une
/// barre (onglets, titre) que l'on ne connaît pas, et la grille est posée en
/// bas de la fenêtre avec des cases de cette proportion.
const MAX_RATIO: f64 = 2.5;

/// Ce que le terminal dit de son curseur : sa ligne et sa colonne, et la
/// taille de la grille (0 si `easytab-term` ne l'envoie pas).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hint {
    pub row: usize,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
}

impl From<&View> for Hint {
    fn from(view: &View) -> Self {
        Self {
            row: view.cursor_row,
            col: view.cursor_col,
            cols: view.term_cols,
            rows: view.term_rows,
        }
    }
}

/// Case du curseur et largeur d'une case, déduites du cadre de la fenêtre du
/// terminal (`frame`) dont les `top_inset` premiers pixels sont la barre de
/// titre. `None` si la taille du terminal est inconnue ou la fenêtre
/// invraisemblable.
pub fn caret_from_frame(frame: Rect, top_inset: i32, hint: Hint) -> Option<(Rect, f64)> {
    if hint.cols == 0 || hint.rows == 0 || hint.row >= hint.rows {
        return None;
    }
    let width = (frame.right - frame.left) as f64;
    let height = (frame.bottom - frame.top - top_inset.max(0)) as f64;
    let cell_width = width / hint.cols as f64;
    let cell_height = (height / hint.rows as f64).min(cell_width * MAX_RATIO);
    let credible = |size: f64| (1.0..=MAX_CELL).contains(&size);
    if !credible(cell_width) || !credible(cell_height) {
        return None;
    }
    // Comptée depuis le bas : les terminaux ont peu de marge en bas, alors
    // qu'en haut barres de titre et d'onglets varient.
    let top = frame.bottom as f64 - (hint.rows - hint.row) as f64 * cell_height;
    let left = frame.left as f64 + hint.col.min(hint.cols) as f64 * cell_width;
    let rect = Rect {
        left: left.round() as i32,
        top: top.round() as i32,
        right: (left + cell_width).round() as i32,
        bottom: (top + cell_height).round() as i32,
    };
    Some((rect, cell_width))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Rect = Rect {
        left: 100,
        top: 50,
        right: 900,
        bottom: 528,
    };

    fn hint(row: usize, col: usize) -> Hint {
        Hint {
            row,
            col,
            cols: 100,
            rows: 24,
        }
    }

    #[test]
    fn places_the_cursor_cell_in_the_grid() {
        // 800 px pour 100 colonnes, 456 px sous une barre de 22 px pour 24
        // lignes : cases de 8 × 19 px.
        let (rect, cell) = caret_from_frame(FRAME, 22, hint(0, 0)).unwrap();
        assert_eq!(cell, 8.0);
        assert_eq!(
            rect,
            Rect {
                left: 100,
                top: 72,
                right: 108,
                bottom: 91
            }
        );
        let (rect, _) = caret_from_frame(FRAME, 22, hint(23, 10)).unwrap();
        assert_eq!(
            rect,
            Rect {
                left: 180,
                top: 509,
                right: 188,
                bottom: 528
            }
        );
    }

    #[test]
    fn keeps_cells_plausible_under_an_unknown_bar() {
        // Barre d'onglets inconnue (inset 0) et grille peu haute : cases de
        // 8 × 20 px au plus, posées en bas de la fenêtre.
        let frame = Rect {
            bottom: 50 + 400,
            ..FRAME
        };
        let hint = Hint {
            rows: 10,
            ..hint(9, 0)
        };
        let (rect, _) = caret_from_frame(frame, 0, hint).unwrap();
        assert_eq!((rect.top, rect.bottom), (430, 450));
    }

    #[test]
    fn needs_the_terminal_size() {
        let unknown = Hint {
            cols: 0,
            rows: 0,
            ..hint(3, 4)
        };
        assert_eq!(caret_from_frame(FRAME, 22, unknown), None);
        // Curseur hors de la grille annoncée.
        assert_eq!(caret_from_frame(FRAME, 22, hint(24, 0)), None);
        // Fenêtre réduite à rien.
        let tiny = Rect {
            right: 150,
            ..FRAME
        };
        assert_eq!(caret_from_frame(tiny, 22, hint(0, 0)), None);
    }

    #[test]
    fn reads_the_hint_from_the_view() {
        let view = View {
            rows: Vec::new(),
            selected: 0,
            description: None,
            word_width: 0,
            cursor_row: 3,
            cursor_col: 7,
            total: 0,
            first: 0,
            term_cols: 80,
            term_rows: 24,
        };
        assert_eq!(
            Hint::from(&view),
            Hint {
                row: 3,
                col: 7,
                cols: 80,
                rows: 24
            }
        );
    }
}
