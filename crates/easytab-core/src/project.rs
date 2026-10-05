//! Valeurs propres au projet, lues directement dans ses fichiers, là où les
//! specs Fig ne proposent rien ou demandent des outils absents (`bash`, `cat`
//! sous Windows) :
//!
//! - `make <cible>` : cibles du `Makefile` ;
//! - `composer run-script <script>` (ou `run`) : scripts de `composer.json` ;
//! - `ng build <projet>` (`serve`, `test`…) : projets de `angular.json` ;
//! - `npm run <script>`, `yarn [run] <script>`, `pnpm [run] <script>`,
//!   `bun run <script>` : scripts de `package.json` ;
//! - `ssh <hôte>`, `sftp <hôte>`, `scp <hôte>:` : hôtes de `~/.ssh/config` (et
//!   des fichiers qu'il inclut) et de `~/.ssh/known_hosts`.
//!
//! Les fichiers sont lus pendant la frappe (ils sont petits) et gardés en
//! mémoire tant que leur date de modification ne change pas.

use std::collections::{HashMap, HashSet};
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
    PackageScripts,
    /// Hôtes SSH ; `colon` : suivis de `:` (`scp hôte:chemin`).
    SshHosts {
        colon: bool,
    },
}

impl Source {
    /// Seule la partie du mot après ce texte est complétée : `ssh marc@hô`.
    pub(crate) fn query_term(self) -> Option<&'static str> {
        match self {
            Source::SshHosts { .. } => Some("@"),
            _ => None,
        }
    }
}

/// Fichier lu, selon son format (clé du cache avec son chemin).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Format {
    Make,
    Composer,
    Angular,
    Package,
    SshConfig,
    KnownHosts,
}

/// Contenu utile d'un fichier.
#[derive(Debug, Default)]
struct Parsed {
    items: Vec<Item>,
    /// `Include` d'un fichier de configuration SSH.
    includes: Vec<String>,
}

/// Fichiers SSH lus au plus, inclusions comprises (protège des boucles).
const MAX_INCLUDES: usize = 16;

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
        // `npm run` (`run-script` en est un alias dans la spec).
        (["npm", "run"], []) => Some(Source::PackageScripts),
        (["yarn" | "pnpm"], []) | (["yarn" | "pnpm" | "bun", "run"], []) => {
            Some(Source::PackageScripts)
        }
        (["ssh" | "sftp"], []) => Some(Source::SshHosts { colon: false }),
        // Chaque argument de scp peut être distant.
        (["scp"], _) => Some(Source::SshHosts { colon: true }),
        _ => None,
    }
}

/// Suggestions de `source` pour la ligne `tokens` (du nom de la commande au
/// mot en cours), dans le dossier `cwd` ; `home` : dossier de l'utilisateur.
pub(crate) fn items(
    source: Source,
    cwd: &Path,
    home: Option<&Path>,
    tokens: &[String],
) -> Arc<Vec<Item>> {
    let format = match source {
        Source::MakeTargets => Format::Make,
        Source::ComposerScripts => Format::Composer,
        Source::AngularProjects => Format::Angular,
        Source::PackageScripts => Format::Package,
        Source::SshHosts { colon } => {
            // `scp hôte:chemin`, `scp ./fichier` : ce n'est plus un hôte.
            let current = tokens.last().map_or("", String::as_str);
            if current.contains([':', '/', '\\']) {
                return Arc::default();
            }
            return Arc::new(home.map(|home| ssh_hosts(home, colon)).unwrap_or_default());
        }
    };
    let Some(file) = find_file(source, cwd, tokens) else {
        return Arc::default();
    };
    match read(format, file) {
        Some(parsed) => Arc::new(parsed.items.clone()),
        None => Arc::default(),
    }
}

/// Contenu de `file` lu selon `format`, gardé en mémoire tant que sa date de
/// modification ne change pas. `None` si le fichier n'existe pas.
fn read(format: Format, file: PathBuf) -> Option<Arc<Parsed>> {
    let modified = std::fs::metadata(&file).and_then(|m| m.modified()).ok()?;
    type Files = Mutex<HashMap<(Format, PathBuf), (SystemTime, Arc<Parsed>)>>;
    static CACHE: OnceLock<Files> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    let key = (format, file);
    if let Some((time, parsed)) = cache.lock().unwrap().get(&key) {
        if *time == modified {
            return Some(Arc::clone(parsed));
        }
    }
    let text = std::fs::read_to_string(&key.1).unwrap_or_default();
    let parsed = Arc::new(match format {
        Format::Make => items_only(make_targets(&text)),
        Format::Composer => items_only(composer_scripts(&text)),
        Format::Angular => items_only(angular_projects(&text)),
        Format::Package => items_only(package_scripts(&text)),
        Format::SshConfig => {
            let (items, includes) = ssh_config(&text);
            Parsed { items, includes }
        }
        Format::KnownHosts => items_only(known_hosts(&text)),
    });
    let mut cache = cache.lock().unwrap();
    if cache.len() >= MAX_FILES {
        cache.clear();
    }
    cache.insert(key, (modified, Arc::clone(&parsed)));
    Some(parsed)
}

fn items_only(items: Vec<Item>) -> Parsed {
    Parsed {
        items,
        includes: Vec::new(),
    }
}

/// Hôtes de `~/.ssh/config` (et des fichiers qu'il inclut), puis de
/// `~/.ssh/known_hosts`, sans doublons ; suivis de `:` si `colon`.
fn ssh_hosts(home: &Path, colon: bool) -> Vec<Item> {
    let ssh = home.join(".ssh");
    // Pile des fichiers à lire : le dernier est lu en premier.
    let mut files = vec![ssh.join("config")];
    let mut visited = HashSet::new();
    let mut hosts: Vec<Item> = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |items: &[Item], hosts: &mut Vec<Item>| {
        for item in items {
            if seen.insert(item.names[0].clone()) {
                hosts.push(item.clone());
            }
        }
    };
    while let Some(file) = files.pop() {
        if visited.len() >= MAX_INCLUDES || !visited.insert(file.clone()) {
            continue;
        }
        let Some(parsed) = read(Format::SshConfig, file) else {
            continue;
        };
        add(&parsed.items, &mut hosts);
        for pattern in parsed.includes.iter().rev() {
            files.extend(expand_include(pattern, home, &ssh).into_iter().rev());
        }
    }
    if let Some(parsed) = read(Format::KnownHosts, ssh.join("known_hosts")) {
        add(&parsed.items, &mut hosts);
    }
    if colon {
        for host in &mut hosts {
            host.names[0].push(':');
        }
    }
    hosts
}

/// Fichiers désignés par un `Include` : chemin relatif à `~/.ssh`, `~/`
/// remplacé par le dossier de l'utilisateur, `*` et `?` acceptés dans le nom
/// du fichier (fichiers trouvés triés, comme ssh).
fn expand_include(pattern: &str, home: &Path, ssh: &Path) -> Vec<PathBuf> {
    let path = match pattern.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => ssh.join(pattern),
    };
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    if !name.contains(['*', '?']) {
        return vec![path];
    }
    let Some(entries) = path.parent().and_then(|dir| std::fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| wildcard(name, n)))
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

/// `text` correspond au motif `pattern` (`*` : n'importe quelle suite, `?` :
/// un caractère).
fn wildcard(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    // Dernière étoile rencontrée et position du texte où elle reprend.
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
            }
            Some(&c) if c == '?' || c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    star = Some((sp, st + 1));
                    p = sp + 1;
                    t = st + 1;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
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
        // npm, yarn… cherchent `package.json` dans les dossiers parents.
        Source::PackageScripts => cwd
            .ancestors()
            .map(|dir| dir.join("package.json"))
            .find(|f| f.is_file()),
        // `ng` cherche l'espace de travail dans les dossiers parents.
        Source::AngularProjects => cwd
            .ancestors()
            .map(|dir| dir.join("angular.json"))
            .find(|f| f.is_file()),
        Source::SshHosts { .. } => None,
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

/// Scripts de `package.json` (clés de `scripts`), décrits par la commande
/// lancée.
pub(crate) fn package_scripts(text: &str) -> Vec<Item> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    scripts
        .iter()
        .map(|(name, command)| {
            let command = command.as_str().filter(|c| !c.is_empty());
            item(name, command.map(str::to_string))
        })
        .collect()
}

/// Motif d'hôte SSH qui n'est pas un nom d'hôte (`*.exemple.org`, `!prod`).
fn is_host_pattern(host: &str) -> bool {
    host.is_empty() || host.contains(['*', '?', '!'])
}

/// Hôtes d'un fichier de configuration SSH (lignes `Host`, hors motifs),
/// décrits par leur `HostName`, et chemins de ses lignes `Include`.
pub(crate) fn ssh_config(text: &str) -> (Vec<Item>, Vec<String>) {
    let mut items: Vec<Item> = Vec::new();
    let mut includes = Vec::new();
    // Hôtes du bloc `Host` en cours (indices dans `items`).
    let mut block: Vec<usize> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `Mot valeur`, `Mot=valeur` ou `Mot = valeur`.
        let (keyword, value) = match line.find(|c: char| c.is_whitespace() || c == '=') {
            Some(i) => (&line[..i], line[i..].trim_start_matches([' ', '\t', '='])),
            None => (line, ""),
        };
        let words = value
            .split_whitespace()
            .map(|w| w.trim_matches('"'))
            .filter(|w| !w.is_empty());
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                block.clear();
                for host in words {
                    if is_host_pattern(host) || items.iter().any(|i| i.names[0] == host) {
                        continue;
                    }
                    block.push(items.len());
                    items.push(item(host, None));
                }
            }
            "match" => block.clear(),
            "hostname" => {
                for &i in &block {
                    if items[i].description.is_none() {
                        items[i].description = Some(value.trim_matches('"').to_string());
                    }
                }
            }
            "include" => includes.extend(words.map(str::to_string)),
            _ => {}
        }
    }
    (items, includes)
}

/// Hôtes de `known_hosts` : premier champ de chaque ligne, séparé par des
/// virgules, sans les entrées hachées (`|1|…`), les marqueurs
/// (`@cert-authority`) ni les motifs ; `[hôte]:port` donne `hôte`.
pub(crate) fn known_hosts(text: &str) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut seen = HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with(['#', '@', '|']) {
            continue;
        }
        let Some(field) = line.split_whitespace().next() else {
            continue;
        };
        for host in field.split(',') {
            let host = match host.strip_prefix('[') {
                Some(rest) => rest.split_once(']').map_or(rest, |(host, _)| host),
                None => host,
            };
            if is_host_pattern(host) || host.starts_with('|') {
                continue;
            }
            if seen.insert(host) {
                items.push(item(host, Some("Hôte connu".to_string())));
            }
        }
    }
    items
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
    fn reads_package_scripts() {
        let items = package_scripts(
            r#"{"name": "app", "scripts": {"test": "vitest", "build": "vite build",
                "vide": "", "bizarre": 3}}"#,
        );
        // Ordre alphabétique (serde_json ne garde pas celui du fichier).
        assert_eq!(names(&items), ["bizarre", "build", "test", "vide"]);
        assert_eq!(items[1].description.as_deref(), Some("vite build"));
        assert_eq!(items[0].description, None);
        assert_eq!(items[3].description, None);
        assert!(package_scripts(r#"{"name": "app"}"#).is_empty());
        assert!(package_scripts("pas du json").is_empty());
    }

    #[test]
    fn reads_ssh_config() {
        let (items, includes) = ssh_config(
            "# Commentaire\n\
             Include config.d/*  ~/autre.conf\n\
             Host prod prod-bis *.interne !secret\n\
             \tHostName 10.0.0.1\n\
             \tUser marc\n\
             host=\"guillemets\"\n\
             \tHostname = exemple.org\n\
             Host ?ouvert\n\
             Match host prod\n\
             \tHostName ignore\n\
             Host *\n\
             \tServerAliveInterval 60\n\
             Host prod\n",
        );
        assert_eq!(names(&items), ["prod", "prod-bis", "guillemets"]);
        assert_eq!(items[0].description.as_deref(), Some("10.0.0.1"));
        assert_eq!(items[1].description.as_deref(), Some("10.0.0.1"));
        assert_eq!(items[2].description.as_deref(), Some("exemple.org"));
        assert_eq!(includes, ["config.d/*", "~/autre.conf"]);
    }

    #[test]
    fn reads_known_hosts() {
        let items = known_hosts(
            "github.com,140.82.121.4 ssh-ed25519 AAAA\n\
             # commentaire\n\
             |1|aGFjaGU=|c2Vs ssh-rsa AAAA\n\
             [git.exemple.org]:2222 ssh-rsa AAAA\n\
             @cert-authority *.exemple.org ssh-rsa AAAA\n\
             *.motif.org ssh-rsa AAAA\n\
             \n\
             github.com ssh-rsa AAAA\n",
        );
        assert_eq!(
            names(&items),
            ["github.com", "140.82.121.4", "git.exemple.org"]
        );
    }

    #[test]
    fn matches_wildcards() {
        assert!(wildcard("*", "config"));
        assert!(wildcard("*.conf", "a.conf"));
        assert!(!wildcard("*.conf", "a.conf.bak"));
        assert!(wildcard("h?te*", "hote-1"));
        assert!(wildcard("a*b*c", "axxbyyc"));
        assert!(!wildcard("a*b*c", "axxbyy"));
        assert!(!wildcard("?", ""));
    }

    #[test]
    fn reads_ssh_hosts_of_the_home_directory() {
        let home = std::env::temp_dir().join(format!("easytab-ssh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(ssh.join("config.d")).unwrap();
        // L'inclusion de lui-même ne boucle pas.
        std::fs::write(
            ssh.join("config"),
            "Include config.d/*.conf\nHost principal\nInclude ~/.ssh/config\n",
        )
        .unwrap();
        std::fs::write(ssh.join("config.d/b.conf"), "Host bis\n").unwrap();
        std::fs::write(ssh.join("config.d/a.conf"), "Host alpha principal\n").unwrap();
        std::fs::write(ssh.join("config.d/c.txt"), "Host ignore\n").unwrap();
        std::fs::write(
            ssh.join("known_hosts"),
            "connu.org ssh-ed25519 AAAA\nalpha ssh-ed25519 AAAA\n",
        )
        .unwrap();
        let tokens = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        let ssh_source = Source::SshHosts { colon: false };
        let found = items(ssh_source, &home, Some(&home), &tokens(&["ssh", ""]));
        assert_eq!(names(&found), ["principal", "alpha", "bis", "connu.org"]);
        let found = items(
            Source::SshHosts { colon: true },
            &home,
            Some(&home),
            &tokens(&["scp", "fichier", ""]),
        );
        assert_eq!(
            names(&found),
            ["principal:", "alpha:", "bis:", "connu.org:"]
        );
        // Chemin distant ou local : plus d'hôtes.
        for word in ["principal:", "./a", "C:\\x"] {
            let found = items(ssh_source, &home, Some(&home), &tokens(&["scp", word]));
            assert!(found.is_empty(), "{word}");
        }
        assert!(items(ssh_source, &home, None, &tokens(&["ssh", ""])).is_empty());
        let _ = std::fs::remove_dir_all(&home);
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
        assert_eq!(
            source(&path(&["npm", "run"]), &[]),
            Some(Source::PackageScripts)
        );
        assert_eq!(source(&path(&["npm", "run"]), &["build"]), None);
        // `npm <script>` n'existe pas.
        assert_eq!(source(&path(&["npm"]), &[]), None);
        for words in [&["yarn"][..], &["yarn", "run"], &["pnpm"], &["pnpm", "run"]] {
            assert_eq!(source(&path(words), &[]), Some(Source::PackageScripts));
        }
        assert_eq!(
            source(&path(&["bun", "run"]), &[]),
            Some(Source::PackageScripts)
        );
        assert_eq!(source(&path(&["bun"]), &[]), None);
        assert_eq!(
            source(&path(&["ssh"]), &[]),
            Some(Source::SshHosts { colon: false })
        );
        // Après l'hôte vient la commande à lancer.
        assert_eq!(source(&path(&["ssh"]), &["serveur"]), None);
        assert_eq!(
            source(&path(&["sftp"]), &[]),
            Some(Source::SshHosts { colon: false })
        );
        assert_eq!(
            source(&path(&["scp"]), &["fichier"]),
            Some(Source::SshHosts { colon: true })
        );
    }

    #[test]
    fn reads_files_of_the_directory_and_follows_changes() {
        let dir = std::env::temp_dir().join(format!("easytab-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sous/dossier")).unwrap();
        std::fs::write(dir.join("Makefile"), "build:\n").unwrap();
        std::fs::write(dir.join("autre.mk"), "deploy:\n").unwrap();
        std::fs::write(dir.join("angular.json"), r#"{"projects": {"app": {}}}"#).unwrap();
        std::fs::write(
            dir.join("sous/package.json"),
            r#"{"scripts": {"dev": "vite"}}"#,
        )
        .unwrap();
        let tokens = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();

        let found = items(Source::MakeTargets, &dir, None, &tokens(&["make", ""]));
        assert_eq!(names(&found), ["build"]);
        let found = items(
            Source::MakeTargets,
            &dir,
            None,
            &tokens(&["make", "-f", "autre.mk", ""]),
        );
        assert_eq!(names(&found), ["deploy"]);
        let found = items(
            Source::MakeTargets,
            &dir.join("sous"),
            None,
            &tokens(&["make", "-C", "..", ""]),
        );
        assert_eq!(names(&found), ["build"]);
        // Pas de Makefile ici.
        assert!(items(
            Source::MakeTargets,
            &dir.join("sous"),
            None,
            &tokens(&["make", ""])
        )
        .is_empty());
        // angular.json est cherché dans les dossiers parents.
        let found = items(
            Source::AngularProjects,
            &dir.join("sous/dossier"),
            None,
            &tokens(&["ng", "build", ""]),
        );
        assert_eq!(names(&found), ["app"]);
        // package.json aussi, le plus proche.
        let found = items(
            Source::PackageScripts,
            &dir.join("sous/dossier"),
            None,
            &tokens(&["npm", "run", ""]),
        );
        assert_eq!(names(&found), ["dev"]);
        assert!(items(
            Source::PackageScripts,
            &dir,
            None,
            &tokens(&["npm", "run", ""])
        )
        .is_empty());

        // Fichier modifié : relu (date de modification différente).
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("Makefile"), "build:\nclean:\n").unwrap();
        let file = std::fs::File::options()
            .write(true)
            .open(dir.join("Makefile"))
            .unwrap();
        file.set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        let found = items(Source::MakeTargets, &dir, None, &tokens(&["make", ""]));
        assert_eq!(names(&found), ["build", "clean"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
