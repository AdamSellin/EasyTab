//! Classement des suggestions : qualité de la correspondance avec le mot tapé
//! (début du nom, puis recherche floue) et fréquence d'utilisation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Nombre maximum de mots retenus dans l'historique d'utilisation.
const MAX_ENTRIES: usize = 5000;

/// Qualité de la correspondance entre `name` et le mot tapé, de 0 (le nom
/// commence par le mot) à 3 (les lettres du mot apparaissent dans l'ordre).
/// `None` si le nom ne correspond pas.
pub fn match_rank(name: &str, query: &str) -> Option<u8> {
    if name.starts_with(query) {
        return Some(0);
    }
    let lower_name = name.to_lowercase();
    let lower_query = query.to_lowercase();
    if lower_name.starts_with(&lower_query) {
        return Some(1);
    }
    // En dessous de deux lettres, la recherche floue proposerait presque tout.
    let len = query.chars().count();
    if len >= 2 && lower_name.contains(&lower_query) {
        return Some(2);
    }
    // Lettres dans l'ordre, la première au début du nom (tirets ignorés) :
    // « chk » trouve « checkout », « --nc » trouve « --no-commit ».
    let bare_query = lower_query.trim_start_matches('-');
    let bare_name = lower_name.trim_start_matches('-');
    if bare_query.chars().count() >= 2
        && bare_name.chars().next() == bare_query.chars().next()
        && is_subsequence(bare_query, bare_name)
    {
        return Some(3);
    }
    None
}

/// Meilleure correspondance parmi plusieurs noms (alias d'une même commande).
pub fn best_match<'a>(names: &'a [String], query: &str) -> Option<(&'a String, u8)> {
    names
        .iter()
        .filter_map(|name| match_rank(name, query).map(|rank| (name, rank)))
        .min_by_key(|&(_, rank)| rank)
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|c| chars.any(|h| h == c))
}

/// Mots tapés dans les commandes exécutées, comptés par contexte (les deux
/// premiers mots qui les précèdent : `git` pour `checkout`, `git checkout`
/// pour une branche). Les mots les plus utilisés remontent dans la liste.
#[derive(Debug, Default)]
pub struct Usage {
    counts: HashMap<String, u32>,
    path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    counts: HashMap<String, u32>,
}

impl Usage {
    /// Historique lu depuis `path` (vide s'il n'existe pas), enregistré à
    /// chaque commande.
    pub fn load(path: &Path) -> Self {
        let counts = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Saved>(&text).ok())
            .map(|saved| saved.counts)
            .unwrap_or_default();
        Self {
            counts,
            path: Some(path.to_path_buf()),
        }
    }

    /// Nombre d'utilisations de `word` après les mots `before`.
    pub fn count(&self, before: &[String], word: &str) -> u32 {
        self.counts
            .get(&key(before, word))
            .copied()
            .unwrap_or_default()
    }

    /// Compte les mots d'une commande exécutée.
    pub fn record(&mut self, words: &[String]) {
        if words.iter().all(String::is_empty) {
            return;
        }
        for i in 0..words.len() {
            if words[i].is_empty() {
                continue;
            }
            *self.counts.entry(key(&words[..i], &words[i])).or_default() += 1;
        }
        if self.counts.len() > MAX_ENTRIES {
            // Oublie les mots utilisés une seule fois.
            self.counts.retain(|_, count| *count > 1);
        }
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let saved = Saved {
            counts: self.counts.clone(),
        };
        if let Ok(text) = serde_json::to_string(&saved) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // Écrit à côté puis renomme : un autre terminal ne lit jamais un
            // fichier à moitié écrit.
            let temp = path.with_extension(format!("tmp{}", std::process::id()));
            if std::fs::write(&temp, text).is_ok() && std::fs::rename(&temp, path).is_err() {
                let _ = std::fs::remove_file(&temp);
            }
        }
    }
}

fn key(before: &[String], word: &str) -> String {
    let context: Vec<&str> = before.iter().take(2).map(String::as_str).collect();
    format!("{}\u{1f}{word}", context.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_prefix_then_fuzzy_matches() {
        assert_eq!(match_rank("checkout", "che"), Some(0));
        assert_eq!(match_rank("Checkout", "che"), Some(1));
        assert_eq!(match_rank("cherry-pick", "pick"), Some(2));
        assert_eq!(match_rank("checkout", "chk"), Some(3));
        assert_eq!(match_rank("commit", "chk"), None);
        assert_eq!(match_rank("--no-commit", "--nc"), Some(3));
        assert_eq!(match_rank("--amend", "--nd"), None);
        // Une seule lettre : seulement le début du nom.
        assert_eq!(match_rank("checkout", "k"), None);
        assert_eq!(match_rank("anything", ""), Some(0));
    }

    #[test]
    fn picks_the_best_alias() {
        let names = vec!["remove".to_string(), "rm".to_string()];
        assert_eq!(best_match(&names, "rm"), Some((&names[1], 0)));
    }

    #[test]
    fn counts_words_by_context_and_persists_them() {
        let dir = std::env::temp_dir().join(format!("easytab-usage-{}", std::process::id()));
        let path = dir.join("usage.json");
        let words = |line: &str| line.split(' ').map(String::from).collect::<Vec<_>>();

        let mut usage = Usage::load(&path);
        usage.record(&words("git checkout main"));
        usage.record(&words("git checkout main"));
        usage.record(&words("git commit"));

        let usage = Usage::load(&path);
        assert_eq!(usage.count(&words("git"), "checkout"), 2);
        assert_eq!(usage.count(&words("git checkout"), "main"), 2);
        assert_eq!(usage.count(&words("git"), "commit"), 1);
        assert_eq!(usage.count(&[], "git"), 3);
        assert_eq!(usage.count(&words("git"), "push"), 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
