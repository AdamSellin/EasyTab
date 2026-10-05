//! Commandes sans spec : leurs options sont lues dans `<commande> --help`.
//!
//! La commande doit être installée (dans le PATH). `--help` est lancé une
//! seule fois, en arrière-plan, depuis le dossier temporaire et sans entrée ;
//! la réponse est gardée dans `~/.easytab/cache/help.json`, par chemin du
//! programme et date de modification (une mise à jour du programme la fait
//! redemander). Elle ne sert que si elle ressemble à une aide : code de sortie
//! 0 ou 1 et au moins deux lignes d'options. Certaines commandes ne sont
//! jamais lancées ainsi (`rm`, `shutdown`…).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::exec;
use crate::spec::{Arg, Command, Opt, Template};

/// Durée maximale de `--help`.
const TIMEOUT: Duration = Duration::from_secs(3);
/// Une aide en a au moins autant.
const MIN_OPTIONS: usize = 2;

/// Jamais lancées avec `--help` : dangereuses si elles l'ignorent.
const DENYLIST: &[&str] = &[
    "shutdown", "reboot", "rm", "dd", "mkfs", "format", "halt", "poweroff", "init", "telinit",
];

/// Option lue dans une aide.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpOpt {
    pub names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Nom de la valeur attendue (`FILE`, `<n>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arg: Option<String>,
    /// La valeur est facultative (`--color[=WHEN]`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    /// La valeur s'écrit collée : `--file=FILE`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub equals: bool,
}

/// Réponse gardée pour un programme.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Entry {
    /// Date de modification du programme, en secondes depuis 1970.
    modified: u64,
    /// Vide si la sortie n'était pas une aide.
    options: Vec<HelpOpt>,
}

#[derive(Default)]
struct State {
    /// Par chemin du programme.
    cache: HashMap<String, Entry>,
    /// Programme trouvé pour chaque nom (`None` : pas dans le PATH), et sa date.
    resolved: HashMap<String, Option<(PathBuf, u64)>>,
    specs: HashMap<String, &'static Command>,
    running: HashSet<String>,
}

#[derive(Clone)]
pub struct HelpSpecs {
    cache_file: Option<PathBuf>,
    /// Dossiers où chercher les programmes (tests), sinon le PATH.
    search: Option<Vec<PathBuf>>,
    state: Arc<Mutex<State>>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl HelpSpecs {
    /// `notify` est appelé quand une nouvelle aide a été lue.
    pub fn start(notify: impl Fn() + Send + Sync + 'static) -> Self {
        let cache_file = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(|home| {
                PathBuf::from(home)
                    .join(".easytab")
                    .join("cache")
                    .join("help.json")
            });
        let cache = cache_file
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        Self {
            cache_file,
            search: None,
            state: Arc::new(Mutex::new(State {
                cache,
                ..State::default()
            })),
            notify: Arc::new(notify),
        }
    }

    /// Cherche les programmes dans `dirs` seulement, sans cache sur disque.
    #[cfg(all(test, unix))]
    pub(crate) fn in_dirs(dirs: Vec<PathBuf>, notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            cache_file: None,
            search: Some(dirs),
            state: Arc::default(),
            notify: Arc::new(notify),
        }
    }

    /// Spec de la commande `name` tirée de son aide. `None` si elle n'en a
    /// pas, ou tant que l'aide est en cours de lecture (son arrivée déclenche
    /// `notify`).
    pub fn spec(&self, name: &str) -> Option<&'static Command> {
        if !allowed(name) {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        let resolved = match state.resolved.get(name) {
            Some(resolved) => resolved.clone(),
            None => {
                let resolved = self.resolve(name);
                state.resolved.insert(name.to_string(), resolved.clone());
                resolved
            }
        };
        let (program, modified) = resolved?;
        let key = program.to_string_lossy().into_owned();
        match state.cache.get(&key) {
            Some(entry) if entry.modified == modified => {
                if entry.options.is_empty() {
                    return None;
                }
                if let Some(spec) = state.specs.get(&key) {
                    return Some(spec);
                }
                let spec: &'static Command =
                    Box::leak(Box::new(command_spec(name, &entry.options)));
                state.specs.insert(key, spec);
                Some(spec)
            }
            _ => {
                if state.running.insert(key.clone()) {
                    drop(state);
                    self.fetch(program, key, modified);
                }
                None
            }
        }
    }

    /// Programme `name` du PATH et sa date de modification.
    fn resolve(&self, name: &str) -> Option<(PathBuf, u64)> {
        let program = match &self.search {
            Some(dirs) => dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()),
            None => exec::find_in_path(name),
        }?;
        // Sous Windows, un script `.bat` ou `.cmd` peut faire n'importe quoi de
        // ses arguments.
        let extension = program
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        if matches!(extension.as_deref(), Some("bat" | "cmd")) {
            return None;
        }
        let modified = std::fs::metadata(&program)
            .and_then(|m| m.modified())
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Some((program, modified))
    }

    /// Lance `--help` en arrière-plan.
    fn fetch(&self, program: PathBuf, key: String, modified: u64) {
        let help = self.clone();
        thread::spawn(move || {
            let output = exec::run(
                &program.to_string_lossy(),
                &["--help".to_string()],
                &std::env::temp_dir(),
                TIMEOUT,
            );
            let options = if matches!(output.status, 0 | 1) {
                // L'aide va parfois sur la sortie d'erreur.
                let mut options = parse_help(&output.stdout);
                if options.len() < MIN_OPTIONS {
                    options = parse_help(&output.stderr);
                }
                options
            } else {
                Vec::new()
            };
            let options = if options.len() >= MIN_OPTIONS {
                options
            } else {
                Vec::new()
            };
            {
                let mut state = help.state.lock().unwrap();
                state.running.remove(&key);
                state.specs.remove(&key);
                state.cache.insert(key, Entry { modified, options });
                if let Some(path) = &help.cache_file {
                    save(path, &state.cache);
                }
            }
            (help.notify)();
        });
    }
}

/// Noms de commande simples, hors liste des commandes dangereuses.
fn allowed(name: &str) -> bool {
    let simple = !name.is_empty()
        && !name.starts_with(['-', '.'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'));
    let lower = name.to_ascii_lowercase();
    let base = lower.strip_suffix(".exe").unwrap_or(&lower);
    // `mkfs.ext4`, `mkfs.vfat`…
    let family = base.split('.').next().unwrap_or(base);
    simple && !DENYLIST.contains(&base) && !DENYLIST.contains(&family)
}

fn save(path: &Path, cache: &HashMap<String, Entry>) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_vec(cache) {
        let _ = std::fs::write(path, data);
    }
}

/// Options d'une aide : lignes `-x, --long-option[=VALUE]  description`
/// (GNU) et variantes courantes (`--opt <valeur>`, `-o VALUE`, description à
/// la ligne suivante comme chez clap, `--[no-]option`).
pub fn parse_help(text: &str) -> Vec<HelpOpt> {
    let lines: Vec<&str> = text.lines().collect();
    let mut options: Vec<HelpOpt> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('-') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let Some(mut option) = parse_line(trimmed) else {
            continue;
        };
        // Description à la ligne suivante, plus en retrait.
        if option.description.is_none() {
            if let Some(next) = lines.get(i + 1) {
                let text = next.trim_start();
                if !text.is_empty() && !text.starts_with('-') && next.len() - text.len() > indent {
                    option.description = Some(text.trim_end().to_string());
                }
            }
        }
        if options
            .iter()
            .any(|o| o.names.iter().any(|n| option.names.contains(n)))
        {
            continue;
        }
        options.push(option);
    }
    options
}

/// Une ligne qui commence par `-`.
fn parse_line(line: &str) -> Option<HelpOpt> {
    // La description suit deux espaces ou une tabulation.
    let (spec, description) = match line.find("  ").into_iter().chain(line.find('\t')).min() {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    };
    let mut option = HelpOpt::default();
    let mut words = spec.split_whitespace().peekable();
    let mut rest: Vec<&str> = Vec::new();
    let mut expects_name = true;
    while let Some(word) = words.next() {
        let separated = word.ends_with(',') || word.ends_with('|');
        let word = word.trim_end_matches([',', '|', ':', ';']);
        if word.starts_with('-') && (expects_name || option.arg.is_none()) {
            let (names, arg) = parse_name(word)?;
            option.names.extend(names);
            if let Some((arg, optional, equals)) = arg {
                option.arg = Some(arg);
                option.optional = optional;
                option.equals = equals;
            }
            expects_name = separated;
            continue;
        }
        if !option.names.is_empty() && is_placeholder(word) {
            // `-o <dir>, --output <dir>` : la même valeur répétée.
            if option.arg.is_none() {
                option.optional = word.starts_with('[');
                option.arg = Some(clean_placeholder(word));
            }
            expects_name = separated;
            continue;
        }
        // Le reste est la description (séparée par une seule espace).
        rest.push(word);
        rest.extend(words.by_ref());
    }
    if option.names.is_empty() {
        return None;
    }
    let description = if rest.is_empty() {
        description.to_string()
    } else {
        format!("{} {description}", rest.join(" "))
    };
    // `-E     : ignore…` (Python).
    let description = description.trim().trim_start_matches(':').trim_start();
    if !description.is_empty() {
        option.description = Some(description.to_string());
    }
    Some(option)
}

type ParsedArg = (String, bool, bool);

/// `--color[=WHEN]`, `--file=FILE`, `-j[N]`, `--[no-]verify`, `-o<file>`.
fn parse_name(word: &str) -> Option<(Vec<String>, Option<ParsedArg>)> {
    let (name, arg) = if let Some(i) = word.find("[=") {
        let arg = word[i + 2..].trim_end_matches(']');
        (&word[..i], Some((clean_placeholder(arg), true, true)))
    } else if let Some((name, arg)) = word.split_once('=') {
        (name, Some((clean_placeholder(arg), false, true)))
    } else if let Some(i) = word.find('<').filter(|&i| i > 1) {
        (
            &word[..i],
            Some((clean_placeholder(&word[i..]), false, false)),
        )
    } else if let Some(i) = word
        .find('[')
        .filter(|&i| i > 1 && !word[..i].ends_with('-'))
    {
        (
            &word[..i],
            Some((clean_placeholder(&word[i..]), true, false)),
        )
    } else {
        (word, None)
    };
    let names = match name.split_once("[no-]") {
        Some((dashes, rest)) => vec![format!("{dashes}{rest}"), format!("{dashes}no-{rest}")],
        None => vec![name.to_string()],
    };
    names.iter().all(|n| valid_name(n)).then_some((names, arg))
}

fn valid_name(name: &str) -> bool {
    let body = name
        .strip_prefix("--")
        .or_else(|| name.strip_prefix('-'))
        .unwrap_or("");
    let mut chars = body.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '?')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `FILE`, `<file>`, `[N]`, `{auto,never}`, `always|never`.
fn is_placeholder(word: &str) -> bool {
    let bracketed = (word.starts_with('<') && word.ends_with('>'))
        || (word.starts_with('[') && word.ends_with(']'))
        || (word.starts_with('{') && word.ends_with('}'));
    let upper = word.chars().any(|c| c.is_ascii_uppercase())
        && word
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '_' | '-'));
    bracketed || upper || is_choice_list(word)
}

/// `always|default|never`.
fn is_choice_list(word: &str) -> bool {
    word.contains('|')
        && word
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '|'))
}

fn clean_placeholder(word: &str) -> String {
    word.trim_matches(['<', '>', '[', ']', '=']).to_string()
}

/// Spec construite à partir des options : des chemins en arguments, comme le
/// shell par défaut.
pub fn command_spec(name: &str, options: &[HelpOpt]) -> Command {
    Command {
        names: vec![name.to_string()],
        options: options
            .iter()
            .map(|option| Opt {
                names: option.names.clone(),
                description: option.description.clone(),
                args: option
                    .arg
                    .iter()
                    .map(|arg| Arg {
                        name: Some(arg.to_lowercase()),
                        optional: option.optional,
                        suggestions: choices(arg),
                        templates: templates(arg),
                        ..Arg::default()
                    })
                    .collect(),
                requires_equals: option.equals,
                ..Opt::default()
            })
            .collect(),
        args: vec![Arg {
            optional: true,
            variadic: true,
            templates: vec![Template::Filepaths],
            ..Arg::default()
        }],
        ..Command::default()
    }
}

/// `{auto,always,never}`, `auto|always|never` : valeurs permises.
fn choices(arg: &str) -> Vec<crate::spec::Value> {
    let braced = arg.strip_prefix('{').and_then(|a| a.strip_suffix('}'));
    let Some(inner) = braced.or_else(|| is_choice_list(arg).then_some(arg)) else {
        return Vec::new();
    };
    inner
        .split([',', '|'])
        .filter(|v| !v.is_empty())
        .map(|v| crate::spec::Value {
            names: vec![v.to_string()],
            ..Default::default()
        })
        .collect()
}

fn templates(arg: &str) -> Vec<Template> {
    let lower = arg.to_ascii_lowercase();
    if lower.contains("dir") || lower.contains("folder") {
        vec![Template::Folders]
    } else if lower.contains("file") || lower.contains("path") {
        vec![Template::Filepaths]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GNU: &str = "Usage: outil [OPTION]... [FILE]...
Concatène des fichiers.

  -A, --show-all           equivalent to -vET
  -b, --number-nonblank    number nonempty output lines
      --color[=WHEN]       colorize the output; WHEN can be 'always'
  -f, --file=FILE          read patterns from FILE
  -n NUM                   print NUM lines
      --[no-]verify        verify (or not)
  -o <dir>, --output <dir>  output directory
      --help     display this help and exit
  -v --verbose  more output
      --format {json,text}  output format
  -E     : ignore PYTHON* environment variables
  --check never|always  check hashes
";

    #[test]
    fn reads_gnu_help() {
        let options = parse_help(GNU);
        let names: Vec<String> = options.iter().map(|o| o.names.join(",")).collect();
        assert_eq!(
            names,
            [
                "-A,--show-all",
                "-b,--number-nonblank",
                "--color",
                "-f,--file",
                "-n",
                "--verify,--no-verify",
                "-o,--output",
                "--help",
                "-v,--verbose",
                "--format",
                "-E",
                "--check",
            ]
        );
        assert_eq!(
            options[0].description.as_deref(),
            Some("equivalent to -vET")
        );
        assert_eq!(options[0].arg, None);
        let color = &options[2];
        assert_eq!(
            (color.arg.as_deref(), color.optional, color.equals),
            (Some("WHEN"), true, true)
        );
        let file = &options[3];
        assert_eq!((file.arg.as_deref(), file.equals), (Some("FILE"), true));
        assert_eq!(options[4].arg.as_deref(), Some("NUM"));
        assert_eq!(options[4].description.as_deref(), Some("print NUM lines"));
        assert_eq!(options[6].arg.as_deref(), Some("dir"));
        assert_eq!(options[6].description.as_deref(), Some("output directory"));
        assert_eq!(options[9].arg.as_deref(), Some("{json,text}"));
        assert_eq!(
            options[10].description.as_deref(),
            Some("ignore PYTHON* environment variables")
        );
        assert_eq!(choices(options[11].arg.as_deref().unwrap()).len(), 2);
    }

    #[test]
    fn reads_descriptions_on_the_next_line() {
        // Style clap.
        let options = parse_help(
            "Options:\n  -c, --config <FILE>\n          Fichier de configuration\n\n  -q, --quiet\n          Moins de messages\n",
        );
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].names, ["-c", "--config"]);
        assert_eq!(options[0].arg.as_deref(), Some("FILE"));
        assert_eq!(
            options[0].description.as_deref(),
            Some("Fichier de configuration")
        );
        assert_eq!(options[1].description.as_deref(), Some("Moins de messages"));
    }

    #[test]
    fn ignores_text_that_is_not_options() {
        let options = parse_help(
            "Usage: x\n- un élément de liste\n-- fin des options\n----------\nRien à voir\n",
        );
        assert!(options.is_empty(), "{options:?}");
    }

    #[test]
    fn builds_a_spec() {
        let spec = command_spec("outil", &parse_help(GNU));
        let file = spec.options.iter().find(|o| o.names[0] == "-f").unwrap();
        assert!(file.requires_equals);
        assert_eq!(file.args[0].templates, [Template::Filepaths]);
        let output = spec.options.iter().find(|o| o.names[0] == "-o").unwrap();
        assert_eq!(output.args[0].templates, [Template::Folders]);
        let format = spec
            .options
            .iter()
            .find(|o| o.names[0] == "--format")
            .unwrap();
        assert_eq!(format.args[0].suggestions[1].names, ["text"]);
        assert_eq!(spec.args[0].templates, [Template::Filepaths]);
    }

    #[test]
    fn refuses_dangerous_or_odd_names() {
        assert!(allowed("rg"));
        assert!(allowed("g++"));
        for name in [
            "rm",
            "shutdown",
            "mkfs.ext4",
            "format.exe",
            "Reboot",
            "-x",
            "./a",
            "a b",
            "",
        ] {
            assert!(!allowed(name), "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn runs_help_once_in_the_background() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::mpsc;

        let dir = std::env::temp_dir().join(format!("easytab-help-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let count = dir.join("count");
        script(
            "outil-aide",
            &format!("echo x >> '{}'\ncat <<'EOF'\n{GNU}EOF", count.display()),
        );
        script("pas-aide", "echo bonjour");
        script("echoue", "echo '  -a  b'; echo '  -c  d'; exit 2");

        let (ready, notified) = mpsc::channel();
        let help = HelpSpecs::in_dirs(vec![dir.clone()], move || {
            let _ = ready.send(());
        });
        let wait = |name: &str| {
            let mut spec = help.spec(name);
            while spec.is_none() && !help.state.lock().unwrap().running.is_empty() {
                notified
                    .recv_timeout(Duration::from_secs(10))
                    .expect("--help sans réponse");
                spec = help.spec(name);
            }
            spec
        };
        let spec = wait("outil-aide").expect("spec tirée de --help");
        assert!(spec
            .options
            .iter()
            .any(|o| o.names.contains(&"--show-all".into())));
        assert!(wait("pas-aide").is_none());
        // Code de sortie 2 : pas une aide.
        assert!(wait("echoue").is_none());
        // Pas dans les dossiers de recherche : rien n'est lancé.
        assert!(help.spec("introuvable").is_none());
        assert!(help.state.lock().unwrap().running.is_empty());
        // `--help` n'a été lancé qu'une fois.
        help.spec("outil-aide").unwrap();
        assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
