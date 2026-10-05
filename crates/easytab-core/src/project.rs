//! Valeurs propres au projet, lues directement dans ses fichiers, là où les
//! specs Fig ne proposent rien ou demandent des outils absents (`bash`, `cat`
//! sous Windows) :
//!
//! - `make <cible>` : cibles du `Makefile` ;
//! - `composer run-script <script>` (ou `run`) : scripts de `composer.json` ;
//! - `ng build <projet>` (`serve`, `test`…) : projets de `angular.json`.
//!
//! Les fichiers sont lus pendant la frappe (ils sont petits) et gardés en
//! mémoire tant que leur date de modification ne change pas.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use crate::generators::Item;

/// Fichiers gardés en mémoire au plus.
const MAX_FILES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Source {
    MakeTargets,
    ComposerScripts,
    AngularProjects,
}

/// Commandes de `ng` qui prennent un projet en premier argument (et leurs
/// alias).
const NG_PROJECT_COMMANDS: &[&str] = &[
    "build",
    "b",
    "serve",
    "s",
    "dev",
    "test",
    "t",
    "e2e",
    "e",
    "lint",
    "l",
    "deploy",
    "extract-i18n",
    "i18n-extract",
    "xi18n",
];

/// Source des suggestions à la place du mot en cours. `path` : la commande
/// puis les sous-commandes de la spec suivies (`["composer", "run-script"]`) ;
/// `positionals` : les mots qui suivent la dernière, hors options.
pub(crate) fn source(path: &[String], positionals: &[&str]) -> Option<Source> {
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    let is_run = |word: &str| matches!(word, "run-script" | "run");
    match (path.as_slice(), positionals) {
        (["make"], _) => Some(Source::MakeTargets),
        // `run-script` n'est une sous-commande que si `composer list` a répondu,
        // et son alias `run` jamais : sinon ce sont des mots ordinaires.
        (["composer", sub], []) if is_run(sub) => Some(Source::ComposerScripts),
        (["composer"], [sub]) if is_run(sub) => Some(Source::ComposerScripts),
        (["ng", sub], []) if NG_PROJECT_COMMANDS.contains(sub) => Some(Source::AngularProjects),
        (["ng"], [sub]) if NG_PROJECT_COMMANDS.contains(sub) => Some(Source::AngularProjects),
        _ => None,
    }
}

/// Suggestions de `source` pour la ligne `tokens` (du nom de la commande au
/// mot en cours), dans le dossier `cwd`.
pub(crate) fn items(source: Source, cwd: &Path, tokens: &[String]) -> Arc<Vec<Item>> {
    let Some(file) = find_file(source, cwd, tokens) else {
        return Arc::default();
    };
    let Some(modified) = std::fs::metadata(&file).and_then(|m| m.modified()).ok() else {
        return Arc::default();
    };
    type Files = Mutex<HashMap<(Source, PathBuf), (SystemTime, Arc<Vec<Item>>)>>;
    static CACHE: OnceLock<Files> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    let key = (source, file);
    if let Some((time, items)) = cache.lock().unwrap().get(&key) {
        if *time == modified {
            return Arc::clone(items);
        }
    }
    let text = std::fs::read_to_string(&key.1).unwrap_or_default();
    let items = Arc::new(match source {
        Source::MakeTargets => make_targets(&text),
        Source::ComposerScripts => composer_scripts(&text),
        Source::AngularProjects => angular_projects(&text),
    });
    let mut cache = cache.lock().unwrap();
    if cache.len() >= MAX_FILES {
        cache.clear();
    }
    cache.insert(key, (modified, Arc::clone(&items)));
    items
}

/// Fichier à lire pour `source`.
fn find_file(source: Source, cwd: &Path, tokens: &[String]) -> Option<PathBuf> {
    match source {
        Source::MakeTargets => {
            // `make -C dossier`, `make -f fichier` : comme make.
            let before = tokens.split_last().map_or(&[][..], |(_, before)| before);
            let mut dir = cwd.to_path_buf();
            let mut file = None;
            for (i, word) in before.iter().enumerate() {
                let next = before.get(i + 1);
                match word.as_str() {
                    "-C" | "--directory" => dir = next.map(|d| cwd.join(d))?,
                    "-f" | "--file" | "--makefile" => file = next.cloned(),
                    _ => {
                        if let Some(d) = word.strip_prefix("--directory=") {
                            dir = cwd.join(d);
                        } else if let Some(f) = word
                            .strip_prefix("--file=")
                            .or_else(|| word.strip_prefix("--makefile="))
                        {
                            file = Some(f.to_string());
                        }
                    }
                }
            }
            match file {
                Some(file) => Some(dir.join(file)).filter(|f| f.is_file()),
                // L'ordre de GNU make.
                None => ["GNUmakefile", "makefile", "Makefile"]
                    .iter()
                    .map(|name| dir.join(name))
                    .find(|f| f.is_file()),
            }
        }
        Source::ComposerScripts => Some(cwd.join("composer.json")).filter(|f| f.is_file()),
        // `ng` cherche l'espace de travail dans les dossiers parents.
        Source::AngularProjects => cwd
            .ancestors()
            .map(|dir| dir.join("angular.json"))
            .find(|f| f.is_file()),
    }
}

fn item(name: &str, description: Option<String>) -> Item {
    Item {
        names: vec![name.to_string()],
        insert: None,
        description,
        icon: None,
        priority: 50,
    }
}

/// Mots-clés de make qui commencent une ligne sans être une règle.
const MAKE_DIRECTIVES: &[&str] = &[
    "ifeq", "ifneq", "ifdef", "ifndef", "else", "endif", "include", "-include", "sinclude",
    "export", "unexport", "override", "private", "vpath", "undefine",
];

/// Cibles d'un Makefile, dans l'ordre du fichier : les lignes `nom: …` qui ne
/// commencent ni par une tabulation (recette) ni par un point (`.PHONY`), hors
/// motifs (`%.o`), variables (`$(OBJ)`) et affectations (`X := a`). La
/// description vient d'un commentaire `## …` en fin de ligne, sinon des
/// lignes `#` juste au-dessus.
pub(crate) fn make_targets(text: &str) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut comment: Vec<&str> = Vec::new();
    let mut in_define = false;
    let mut continued = false;
    for line in text.lines() {
        let was_continued = continued;
        continued = line.ends_with('\\');
        if was_continued || line.starts_with('\t') {
            comment.clear();
            continue;
        }
        let trimmed = line.trim();
        let first = trimmed.split_whitespace().next().unwrap_or("");
        if in_define {
            in_define = first != "endef";
            continue;
        }
        if first == "define" {
            in_define = true;
            continue;
        }
        if let Some(text) = trimmed.strip_prefix('#') {
            comment.push(text.trim_start_matches('#').trim());
            continue;
        }
        let above = std::mem::take(&mut comment);
        if trimmed.is_empty() || trimmed.starts_with('.') || MAKE_DIRECTIVES.contains(&first) {
            continue;
        }
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        let (names, rest) = (&trimmed[..colon], &trimmed[colon + 1..]);
        // `X := a`, `X ::= a`, `X = a:b`, `X ?= a`…
        if names.contains('=') || rest.starts_with('=') || rest.starts_with(":=") {
            continue;
        }
        let description = match rest.split_once("##") {
            Some((_, text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
            _ => above
                .iter()
                .rev()
                .find(|line| !line.is_empty())
                .map(|line| line.to_string()),
        };
        for name in names.split_whitespace() {
            let pattern = name.contains(['%', '$', '(', ')']) || name.starts_with('.');
            if pattern || items.iter().any(|i| i.names[0] == name) {
                continue;
            }
            items.push(item(name, description.clone()));
        }
    }
    items
}

/// Scripts de `composer.json` (clés de `scripts`), décrits par
/// `scripts-descriptions` ou par la commande lancée.
pub(crate) fn composer_scripts(text: &str) -> Vec<Item> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    let descriptions = json.get("scripts-descriptions");
    scripts
        .iter()
        .map(|(name, command)| {
            let described = descriptions
                .and_then(|d| d.get(name))
                .and_then(|d| d.as_str());
            let command = match command {
                serde_json::Value::String(command) => Some(command.clone()),
                serde_json::Value::Array(steps) => Some(
                    steps
                        .iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" && "),
                ),
                _ => None,
            };
            let description = described.map(str::to_string).or(command);
            item(name, description.filter(|d| !d.is_empty()))
        })
        .collect()
}

/// Projets de `angular.json` (clés de `projects`), décrits par leur type
/// (`application`, `library`).
pub(crate) fn angular_projects(text: &str) -> Vec<Item> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(projects) = json.get("projects").and_then(|p| p.as_object()) else {
        return Vec::new();
    };
    projects
        .iter()
        .map(|(name, project)| {
            let kind = project.get("projectType").and_then(|t| t.as_str());
            item(name, kind.map(str::to_string))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.names[0].as_str()).collect()
    }

    #[test]
    fn reads_make_targets() {
        let items = make_targets(
            "CC := gcc\n\
             PREFIX ?= /usr\n\
             URL = http://exemple.org:80\n\
             .PHONY: build test\n\
             \n\
             # Compile tout\n\
             build: main.o ## Compile le projet\n\
             \t$(CC) -o app main.o\n\
             \n\
             # Lance les tests\n\
             test: build\n\
             \t./app --test\n\
             %.o: %.c\n\
             \t$(CC) -c $<\n\
             $(OBJ): config.h\n\
             install uninstall:\n\
             debug: CFLAGS += -g\n\
             ifeq ($(OS),Windows_NT)\n\
             define AIDE\n\
             faux: cible\n\
             endef\n\
             endif\n\
             build: autre\n\
             long-target: a \\\n\
             \tnot-a-target: b\n",
        );
        assert_eq!(
            names(&items),
            [
                "build",
                "test",
                "install",
                "uninstall",
                "debug",
                "long-target"
            ]
        );
        assert_eq!(items[0].description.as_deref(), Some("Compile le projet"));
        assert_eq!(items[1].description.as_deref(), Some("Lance les tests"));
        assert_eq!(items[2].description, None);
    }

    #[test]
    fn reads_composer_scripts() {
        let items = composer_scripts(
            r#"{"name": "a/b", "scripts": {"test": "phpunit", "lint": ["phpcs", "phpstan"],
                "post-install-cmd": "@php artisan"},
                "scripts-descriptions": {"test": "Lance les tests"}}"#,
        );
        // Ordre alphabétique (serde_json ne garde pas celui du fichier).
        assert_eq!(names(&items), ["lint", "post-install-cmd", "test"]);
        assert_eq!(items[2].description.as_deref(), Some("Lance les tests"));
        assert_eq!(items[0].description.as_deref(), Some("phpcs && phpstan"));
        assert!(composer_scripts("{}").is_empty());
        assert!(composer_scripts("pas du json").is_empty());
    }

    #[test]
    fn reads_angular_projects() {
        let items = angular_projects(
            r#"{"version": 1, "projects": {"boutique": {"projectType": "application"},
                "ui-kit": {"projectType": "library"}}}"#,
        );
        assert_eq!(names(&items), ["boutique", "ui-kit"]);
        assert_eq!(items[1].description.as_deref(), Some("library"));
    }

    #[test]
    fn picks_the_source_from_the_words() {
        let path = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        assert_eq!(source(&path(&["make"]), &[]), Some(Source::MakeTargets));
        assert_eq!(
            source(&path(&["make"]), &["build"]),
            Some(Source::MakeTargets)
        );
        assert_eq!(
            source(&path(&["composer", "run-script"]), &[]),
            Some(Source::ComposerScripts)
        );
        assert_eq!(
            source(&path(&["composer"]), &["run"]),
            Some(Source::ComposerScripts)
        );
        // `composer <script>` : déjà proposé par la spec Fig (`composer list`).
        assert_eq!(source(&path(&["composer"]), &[]), None);
        assert_eq!(source(&path(&["composer", "run-script"]), &["test"]), None);
        assert_eq!(
            source(&path(&["ng"]), &["build"]),
            Some(Source::AngularProjects)
        );
        assert_eq!(source(&path(&["ng"]), &["new"]), None);
        assert_eq!(source(&path(&["ng"]), &["build", "app"]), None);
        assert_eq!(source(&path(&["npm", "run"]), &[]), None);
    }

    #[test]
    fn reads_files_of_the_directory_and_follows_changes() {
        let dir = std::env::temp_dir().join(format!("easytab-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sous/dossier")).unwrap();
        std::fs::write(dir.join("Makefile"), "build:\n").unwrap();
        std::fs::write(dir.join("autre.mk"), "deploy:\n").unwrap();
        std::fs::write(dir.join("angular.json"), r#"{"projects": {"app": {}}}"#).unwrap();
        let tokens = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();

        let found = items(Source::MakeTargets, &dir, &tokens(&["make", ""]));
        assert_eq!(names(&found), ["build"]);
        let found = items(
            Source::MakeTargets,
            &dir,
            &tokens(&["make", "-f", "autre.mk", ""]),
        );
        assert_eq!(names(&found), ["deploy"]);
        let found = items(
            Source::MakeTargets,
            &dir.join("sous"),
            &tokens(&["make", "-C", "..", ""]),
        );
        assert_eq!(names(&found), ["build"]);
        // Pas de Makefile ici.
        assert!(items(
            Source::MakeTargets,
            &dir.join("sous"),
            &tokens(&["make", ""])
        )
        .is_empty());
        // angular.json est cherché dans les dossiers parents.
        let found = items(
            Source::AngularProjects,
            &dir.join("sous/dossier"),
            &tokens(&["ng", "build", ""]),
        );
        assert_eq!(names(&found), ["app"]);

        // Fichier modifié : relu (date de modification différente).
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("Makefile"), "build:\nclean:\n").unwrap();
        let file = std::fs::File::options()
            .write(true)
            .open(dir.join("Makefile"))
            .unwrap();
        file.set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        let found = items(Source::MakeTargets, &dir, &tokens(&["make", ""]));
        assert_eq!(names(&found), ["build", "clean"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
