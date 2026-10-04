//! Suggestions de fichiers et dossiers (templates `filepaths` et `folders`).

use std::path::{Path, PathBuf};

/// Nombre maximum d'entrées lues dans un dossier.
const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Chemin tel qu'il sera inséré (non échappé), avec `/` final pour un dossier.
    pub path: String,
    /// Nom seul, pour l'affichage.
    pub name: String,
    pub is_dir: bool,
}

/// Entrées qui complètent `typed`, relatif à `cwd` sauf s'il est absolu ou
/// commence par `~/`.
pub fn complete(cwd: &Path, typed: &str, folders_only: bool) -> Vec<Entry> {
    let (dir_part, prefix) = match typed.rfind('/') {
        Some(i) => typed.split_at(i + 1),
        None => ("", typed),
    };
    let Some(dir) = resolve(cwd, dir_part) else {
        return Vec::new();
    };
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut entries: Vec<Entry> = read_dir
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.starts_with(prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            // `Path::is_dir` suit les liens symboliques.
            let is_dir = entry.path().is_dir();
            if folders_only && !is_dir {
                return None;
            }
            let slash = if is_dir { "/" } else { "" };
            Some(Entry {
                path: format!("{dir_part}{name}{slash}"),
                name: format!("{name}{slash}"),
                is_dir,
            })
        })
        .take(MAX_ENTRIES)
        .collect();
    entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));
    entries
}

fn resolve(cwd: &Path, dir_part: &str) -> Option<PathBuf> {
    if dir_part.is_empty() {
        return Some(cwd.to_path_buf());
    }
    if let Some(rest) = dir_part.strip_prefix("~/") {
        return Some(home()?.join(rest));
    }
    Some(cwd.join(dir_part))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Dossier de test propre à chaque test (ils tournent en parallèle).
    fn fixture(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("easytab-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src/bin")).unwrap();
        fs::write(dir.join("src/main.rs"), "").unwrap();
        fs::write(dir.join("README.md"), "").unwrap();
        fs::write(dir.join(".env"), "").unwrap();
        dir
    }

    fn paths(entries: Vec<Entry>) -> Vec<String> {
        entries.into_iter().map(|e| e.path).collect()
    }

    #[test]
    fn lists_folders_first_and_hides_dotfiles() {
        let dir = fixture("hidden");
        assert_eq!(paths(complete(&dir, "", false)), ["src/", "README.md"]);
        assert_eq!(paths(complete(&dir, ".", false)), [".env"]);
        assert_eq!(paths(complete(&dir, "", true)), ["src/"]);
    }

    #[test]
    fn completes_inside_subfolders() {
        let dir = fixture("subfolders");
        assert_eq!(paths(complete(&dir, "src/m", false)), ["src/main.rs"]);
        assert_eq!(
            paths(complete(&dir, "src/", false)),
            ["src/bin/", "src/main.rs"]
        );
        assert!(complete(&dir, "absent/", false).is_empty());
    }
}
