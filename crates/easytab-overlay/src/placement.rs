//! Position de la fenêtre par rapport au curseur du terminal.

/// Rectangle en pixels de l'écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
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

    #[test]
    fn scales_css_pixels() {
        let (x, _, width, height) = place(caret(1000, 300), 18.0, 1, MEASURE, 2.0, SCREEN);
        assert_eq!(x + 80, 1000 - 18);
        assert_eq!((width, height), (600, 400));
    }
}
