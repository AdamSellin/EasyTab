//! Messages échangés entre `easytab-term` et la fenêtre flottante
//! (`easytab-overlay`), une ligne JSON par message.

use serde::{Deserialize, Serialize};

/// De `easytab-term` vers la fenêtre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Affiche (ou met à jour) la liste sous le curseur du terminal.
    Show(View),
    Hide,
}

/// De la fenêtre vers `easytab-term`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// La fenêtre est prête à afficher des listes.
    Ready,
    /// Le curseur du terminal est introuvable : la liste doit être dessinée
    /// dans le terminal.
    Unavailable,
}

/// Ce que la fenêtre affiche : les lignes visibles et la description de la
/// suggestion choisie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub rows: Vec<Row>,
    /// Ligne choisie, parmi `rows`.
    pub selected: usize,
    pub description: Option<String>,
    /// Largeur du mot tapé, en colonnes : les noms s'alignent sous lui.
    pub word_width: usize,
    /// Position du curseur dans le terminal (ligne, colonne), pour déduire
    /// sa position à l'écran entre deux lectures.
    pub cursor_row: usize,
    pub cursor_col: usize,
    /// Taille du terminal (colonnes, lignes), pour estimer la position du
    /// curseur à partir de la fenêtre du terminal quand le système ne la
    /// donne pas (macOS sans accès à l'accessibilité, Linux). Absente des
    /// messages d'une version plus ancienne : 0.
    #[serde(default)]
    pub term_cols: usize,
    #[serde(default)]
    pub term_rows: usize,
    /// Nombre total de suggestions et position de la première ligne visible.
    pub total: usize,
    pub first: usize,
    /// Thème clair (`[list] theme = "light"`).
    #[serde(default)]
    pub light: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub label: String,
    pub hint: Option<String>,
    /// `command`, `subcommand`, `option`, `value`, `folder`, `file` ou `dynamic`.
    pub kind: String,
    /// Icône proposée par la spec (`git`, `npm`…), si elle en donne une.
    pub icon: Option<String>,
    /// Positions (en caractères) des lettres tapées dans `label`.
    pub matched: Vec<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_view_without_terminal_size() {
        // Message d'une version qui n'envoyait pas la taille du terminal.
        let line = r#"{"show":{"rows":[],"selected":0,"description":null,"word_width":2,"cursor_row":3,"cursor_col":7,"total":0,"first":0}}"#;
        let Ok(Request::Show(view)) = serde_json::from_str::<Request>(line) else {
            panic!("message illisible");
        };
        assert_eq!((view.cursor_row, view.cursor_col), (3, 7));
        assert_eq!((view.term_cols, view.term_rows), (0, 0));
    }
}
