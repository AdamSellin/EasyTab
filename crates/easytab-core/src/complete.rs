//! Moteur de suggestions : parcourt la spec de la commande en suivant les mots
//! déjà tapés, puis propose ce qui peut venir à la place du mot en cours.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use crate::exec;
use crate::files;
use crate::generators::{Generators, Item};
use crate::history::History;
use crate::line::{self, Token};
use crate::pwsh::PowerShell;
use crate::rank::{self, best_match, Usage};
use crate::spec::{self, Arg, Command, Generator, Opt, Spec, Template};

/// Nombre maximum de suggestions renvoyées.
const MAX_SUGGESTIONS: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Command,
    Subcommand,
    Option,
    Value,
    Folder,
    File,
    /// Valeur calculée par un generator (branche git, script npm…).
    Dynamic,
    /// Commande entière déjà tapée, tirée de l'historique du shell.
    History,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Texte affiché dans la liste.
    pub label: String,
    /// Texte qui remplace le mot en cours, déjà échappé pour le shell.
    pub insert: String,
    pub description: Option<String>,
    /// Arguments attendus, affichés en gris après le nom (`<mode>`, `[pathspec...]`).
    pub hint: Option<String>,
    /// Icône proposée par la spec (`git`, `npm`…).
    pub icon: Option<String>,
    pub kind: Kind,
    /// Ajouter un espace après l'insertion.
    pub append_space: bool,
    /// Qualité de la correspondance avec le mot tapé : 0 si le nom commence
    /// par lui, plus haut pour la recherche floue (voir [`rank::match_rank`]).
    pub rank: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Mot en cours tel que tapé : c'est lui que l'insertion remplace.
    pub replace: String,
    pub suggestions: Vec<Suggestion>,
    /// Des generators calculent encore des suggestions pour cette ligne.
    pub pending: bool,
}

pub struct Completer {
    commands: Vec<Spec>,
    /// Ne propose comme commandes que celles installées (dans le PATH).
    installed: Option<OnceLock<HashSet<String>>>,
    /// Le shell est PowerShell : ses commandes sont décrites par lui-même.
    powershell: Option<PowerShell>,
    by_name: HashMap<String, usize>,
    generators: Generators,
    usage: Mutex<Usage>,
    history: Mutex<History>,
}

/// Ce que les generators ont besoin de savoir sur la ligne.
struct Context<'a> {
    cwd: &'a Path,
    /// Mots de la commande, du nom de la commande au mot en cours compris.
    tokens: Vec<String>,
    /// Module JS de la spec en cours.
    module: Option<&'a str>,
    pending: bool,
}

impl Completer {
    pub fn new(commands: Vec<Command>) -> Self {
        Self::from_specs(commands.into_iter().map(Spec::from).collect())
    }

    fn from_specs(commands: Vec<Spec>) -> Self {
        let mut by_name = HashMap::new();
        for (i, command) in commands.iter().enumerate() {
            for name in &command.names {
                by_name.entry(name.clone()).or_insert(i);
            }
        }
        Self {
            commands,
            installed: None,
            powershell: None,
            by_name,
            generators: Generators::disabled(),
            usage: Mutex::default(),
            history: Mutex::default(),
        }
    }

    /// Classe aussi les suggestions selon les commandes déjà exécutées.
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Mutex::new(usage);
        self
    }

    /// Propose aussi les commandes déjà tapées.
    pub fn with_history(mut self, history: History) -> Self {
        self.history = Mutex::new(history);
        self
    }

    /// Note les mots d'une commande exécutée.
    pub fn record(&self, input: &str) {
        self.history.lock().unwrap().push(input);
        let line = line::parse(input);
        let words: Vec<String> = line
            .words
            .iter()
            .chain([&line.current])
            .map(|t| t.value.clone())
            .collect();
        self.usage.lock().unwrap().record(&words);
    }

    /// Completer avec les specs embarquées, sans generators.
    pub fn builtin() -> Self {
        let mut completer = Self::from_specs(spec::builtin());
        completer.installed = Some(OnceLock::new());
        completer
    }

    /// Complète aussi les commandes PowerShell (`Remove-Item -Recurse`).
    pub fn with_powershell(mut self, powershell: PowerShell) -> Self {
        self.powershell = Some(powershell);
        self
    }

    /// Active les suggestions dynamiques.
    pub fn with_generators(mut self, generators: Generators) -> Self {
        self.generators = generators;
        self
    }

    pub fn complete(&self, input: &str, cwd: &Path) -> Completion {
        let line = line::parse(input);
        let mut out = Vec::new();
        let mut context = Context {
            cwd,
            tokens: Vec::new(),
            module: None,
            pending: false,
        };
        self.complete_words(&line.words, &line.current, &mut context, &mut out);
        self.rank(&context.tokens, &mut out);
        // Les commandes déjà tapées passent devant, comme dans Fig.
        let typed = input
            .strip_suffix(line.current.raw.as_str())
            .unwrap_or(input);
        let history: Vec<Suggestion> = self
            .history
            .lock()
            .unwrap()
            .matches(input)
            .into_iter()
            .filter_map(|full| {
                let rest = full.trim_start().strip_prefix(typed.trim_start())?;
                Some(Suggestion {
                    label: full.to_string(),
                    insert: rest.to_string(),
                    description: None,
                    hint: None,
                    icon: None,
                    kind: Kind::History,
                    append_space: false,
                    rank: 0,
                })
            })
            .collect();
        out.splice(0..0, history);
        out.truncate(MAX_SUGGESTIONS);
        Completion {
            replace: line.current.raw,
            suggestions: out,
            pending: context.pending,
        }
    }

    /// Trie les suggestions : d'abord la qualité de la correspondance, puis la
    /// fréquence d'utilisation ; à égalité, l'ordre d'origine (alphabétique,
    /// ou celui du generator, comme les branches les plus récentes d'abord).
    fn rank(&self, tokens: &[String], out: &mut [Suggestion]) {
        let before = tokens.split_last().map_or(&[][..], |(_, before)| before);
        let usage = self.usage.lock().unwrap();
        out.sort_by_cached_key(|s| {
            let word = s.insert.trim_end_matches('=');
            (s.rank, std::cmp::Reverse(usage.count(before, word)))
        });
    }

    fn find(&self, name: &str) -> Option<&Command> {
        // `/usr/bin/git` -> `git`
        let name = name.rsplit('/').next().unwrap_or(name);
        self.by_name.get(name).map(|&i| self.commands[i].command())
    }

    fn complete_words<'a>(
        &'a self,
        words: &[Token],
        current: &Token,
        context: &mut Context<'a>,
        out: &mut Vec<Suggestion>,
    ) {
        let Some((name, rest)) = words.split_first() else {
            self.push_commands(&current.value, out);
            return;
        };
        let Some(spec) = self.find(&name.value).or_else(|| {
            self.powershell
                .as_ref()
                .and_then(|powershell| powershell.spec(&name.value))
        }) else {
            return;
        };
        context.module = spec.module.as_deref();
        context.tokens = words
            .iter()
            .chain([current])
            .map(|t| t.value.clone())
            .collect();

        let mut node = spec;
        let mut persistent: Vec<&Opt> = node.options.iter().filter(|o| o.persistent).collect();
        let mut positional = 0;
        let mut seen_positional = false;
        let mut pending: Option<&Arg> = None;
        let mut options_done = false;

        for (i, word) in rest.iter().enumerate() {
            let word = word.value.as_str();
            if pending.take().is_some() {
                continue;
            }
            if !options_done && word == "--" {
                options_done = true;
                continue;
            }
            if !options_done && word.len() > 1 && word.starts_with('-') {
                let (flag, has_value) = match word.split_once('=') {
                    Some((flag, _)) => (flag, true),
                    None => (word, false),
                };
                if let Some(opt) = find_option(node, &persistent, flag) {
                    if !has_value {
                        pending = opt.args.first().filter(|arg| !arg.optional);
                    }
                }
                continue;
            }
            if !seen_positional {
                if let Some(sub) = find_subcommand(node, word) {
                    node = sub;
                    persistent.extend(node.options.iter().filter(|o| o.persistent));
                    positional = 0;
                    continue;
                }
            }
            seen_positional = true;
            if let Some(arg) = node.args.get(positional) {
                if arg.is_command {
                    // `sudo git ch` : on recommence avec la commande imbriquée.
                    return self.complete_words(&rest[i..], current, context, out);
                }
                if !arg.variadic {
                    positional += 1;
                }
            }
        }

        let prefix = current.value.as_str();
        if let Some(arg) = pending {
            self.push_arg(arg, prefix, "", context, out);
            return;
        }
        if !options_done && prefix.starts_with('-') {
            if let Some((flag, value)) = prefix.split_once('=') {
                if let Some(arg) = find_option(node, &persistent, flag).and_then(|o| o.args.first())
                {
                    self.push_arg(arg, value, &format!("{flag}="), context, out);
                }
            } else {
                push_options(node, &persistent, prefix, out);
            }
            return;
        }
        if !seen_positional {
            push_subcommands(node, prefix, out);
        }
        if let Some(arg) = node.args.get(positional) {
            if arg.is_command {
                self.push_commands(prefix, out);
            } else {
                self.push_arg(arg, prefix, "", context, out);
            }
        }
        // Les options ne servent qu'à défaut d'autre chose : pas pendant que des
        // generators calculent.
        if out.is_empty() && prefix.is_empty() && !context.pending {
            push_options(node, &persistent, prefix, out);
        }
    }

    fn push_commands(&self, prefix: &str, out: &mut Vec<Suggestion>) {
        if prefix.is_empty() {
            return;
        }
        let installed = self
            .installed
            .as_ref()
            .map(|lock| lock.get_or_init(exec::installed_programs));
        let mut found: Vec<Suggestion> = self
            .commands
            .iter()
            .flat_map(|c| c.names.iter().map(move |n| (n, c)))
            .filter(|(name, _)| installed.is_none_or(|set| set.contains(name.as_str())))
            .filter_map(|(name, command)| {
                Some(Suggestion {
                    rank: rank::match_rank(name, prefix)?,
                    label: name.clone(),
                    insert: name.clone(),
                    description: command.description.clone(),
                    kind: Kind::Command,
                    hint: None,
                    icon: None,
                    append_space: true,
                })
            })
            .collect();
        if let Some(powershell) = &self.powershell {
            found.extend(powershell.commands().into_iter().filter_map(|name| {
                Some(Suggestion {
                    rank: rank::match_rank(&name, prefix)?,
                    insert: name.clone(),
                    label: name,
                    description: None,
                    kind: Kind::Command,
                    hint: None,
                    icon: None,
                    append_space: true,
                })
            }));
        }
        sort(&mut found);
        found.dedup_by(|a, b| a.label == b.label);
        out.extend(found);
    }

    fn push_arg(
        &self,
        arg: &Arg,
        prefix: &str,
        insert_prefix: &str,
        context: &mut Context,
        out: &mut Vec<Suggestion>,
    ) {
        let mut found: Vec<Suggestion> = arg
            .suggestions
            .iter()
            .filter(|v| !v.hidden)
            .filter_map(|v| {
                let (name, rank) = best_match(&v.names, prefix)?;
                let insert = v.insert.clone().unwrap_or_else(|| line::escape(name));
                Some(Suggestion {
                    rank,
                    label: name.clone(),
                    append_space: !insert.ends_with(['/', '=']),
                    insert: format!("{insert_prefix}{insert}"),
                    description: v.description.clone().or_else(|| arg.description.clone()),
                    kind: Kind::Value,
                    hint: None,
                    icon: None,
                })
            })
            .collect();
        sort(&mut found);
        out.extend(found);

        if let Some(module) = context.module {
            for generator in &arg.generators {
                match self
                    .generators
                    .lookup(module, generator, context.cwd, &context.tokens)
                {
                    Some(items) => push_generated(&items, generator, prefix, insert_prefix, out),
                    None => context.pending = true,
                }
            }
        }

        if arg.templates.is_empty() {
            return;
        }
        let folders_only = !arg.templates.contains(&Template::Filepaths);
        out.extend(
            files::complete(context.cwd, prefix, folders_only)
                .into_iter()
                .map(|entry| Suggestion {
                    label: entry.name,
                    insert: format!("{insert_prefix}{}", line::escape(&entry.path)),
                    description: None,
                    hint: None,
                    icon: None,
                    kind: if entry.is_dir {
                        Kind::Folder
                    } else {
                        Kind::File
                    },
                    append_space: !entry.is_dir,
                    rank: 0,
                }),
        );
    }
}

fn find_subcommand<'a>(node: &'a Command, word: &str) -> Option<&'a Command> {
    node.subcommands
        .iter()
        .find(|s| s.names.iter().any(|n| n == word))
}

fn find_option<'a>(node: &'a Command, persistent: &[&'a Opt], flag: &str) -> Option<&'a Opt> {
    let options = || node.options.iter().chain(persistent.iter().copied());
    options()
        .find(|o| o.names.iter().any(|n| n == flag))
        // PowerShell ignore la casse : `-recurse` vaut `-Recurse`. Pas pour les
        // options d'une lettre, où `-v` et `-V` diffèrent souvent.
        .or_else(|| {
            options().find(|o| {
                o.names
                    .iter()
                    .any(|n| n.len() > 2 && !n.starts_with("--") && n.eq_ignore_ascii_case(flag))
            })
        })
}

fn push_subcommands(node: &Command, prefix: &str, out: &mut Vec<Suggestion>) {
    let mut found: Vec<Suggestion> = node
        .subcommands
        .iter()
        .filter(|s| !s.hidden)
        .filter_map(|s| {
            let (name, rank) = best_match(&s.names, prefix)?;
            Some(Suggestion {
                rank,
                label: name.clone(),
                insert: s.insert.clone().unwrap_or_else(|| name.clone()),
                description: s.description.clone(),
                hint: args_hint(&s.args),
                icon: None,
                kind: Kind::Subcommand,
                append_space: true,
            })
        })
        .collect();
    sort(&mut found);
    out.extend(found);
}

/// Arguments d'une sous-commande ou d'une option, façon Fig :
/// `<mode>` pour un argument obligatoire, `[pathspec...]` sinon.
fn args_hint(args: &[Arg]) -> Option<String> {
    let parts: Vec<String> = args
        .iter()
        .map(|arg| {
            let name = arg.name.as_deref().unwrap_or("arg");
            let dots = if arg.variadic { "..." } else { "" };
            if arg.optional {
                format!("[{name}{dots}]")
            } else {
                format!("<{name}{dots}>")
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn push_options(node: &Command, persistent: &[&Opt], prefix: &str, out: &mut Vec<Suggestion>) {
    let mut seen = HashSet::new();
    let mut found: Vec<Suggestion> = node
        .options
        .iter()
        .chain(persistent.iter().copied())
        .filter(|o| !o.hidden && seen.insert(o.names.first()))
        .filter_map(|o| {
            let (name, rank) = best_match(&o.names, prefix)?;
            let takes_equals = o.requires_equals && !o.args.is_empty();
            let insert = match &o.insert {
                Some(insert) => insert.clone(),
                None if takes_equals => format!("{name}="),
                None => name.clone(),
            };
            Some(Suggestion {
                rank,
                label: o.names.join(", "),
                insert,
                description: o.description.clone(),
                hint: args_hint(&o.args),
                icon: None,
                kind: Kind::Option,
                append_space: !takes_equals,
            })
        })
        .collect();
    sort(&mut found);
    out.extend(found);
}

/// Résultats d'un generator qui complètent le mot en cours.
fn push_generated(
    items: &[Item],
    generator: &Generator,
    prefix: &str,
    insert_prefix: &str,
    out: &mut Vec<Suggestion>,
) {
    // Avec `query_term`, seule la fin du mot est complétée (`origin/ma`).
    let (head, query) = match generator
        .query_term
        .as_deref()
        .and_then(|sep| prefix.rfind(sep).map(|i| i + sep.len()))
    {
        Some(i) => prefix.split_at(i),
        None => ("", prefix),
    };
    let mut seen: HashSet<String> = out.iter().map(|s| s.label.clone()).collect();
    for item in items {
        let Some((name, rank)) = best_match(&item.names, query) else {
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        let value = match &item.insert {
            Some(insert) => strip_cursor(insert),
            None => line::escape(name),
        };
        let insert = format!("{insert_prefix}{}{value}", line::escape(head));
        out.push(Suggestion {
            label: name.clone(),
            append_space: !insert.ends_with(['/', '=']),
            insert,
            description: item.description.clone(),
            hint: None,
            icon: item.icon.clone(),
            kind: Kind::Dynamic,
            rank,
        });
    }
}

/// Retire le marqueur `{cursor}` des valeurs à insérer de Fig.
fn strip_cursor(insert: &str) -> String {
    match insert.find("{cursor") {
        Some(start) => {
            let end = insert[start..]
                .find('}')
                .map_or(insert.len(), |i| start + i + 1);
            format!("{}{}", &insert[..start], &insert[end..])
        }
        None => insert.to_string(),
    }
}

fn sort(found: &mut [Suggestion]) {
    found.sort_by(|a, b| a.label.cmp(&b.label));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn completer() -> &'static Completer {
        use std::sync::OnceLock;
        static COMPLETER: OnceLock<Completer> = OnceLock::new();
        COMPLETER.get_or_init(Completer::builtin)
    }

    fn labels(input: &str) -> Vec<String> {
        completer()
            .complete(input, &PathBuf::from("/nonexistent"))
            .suggestions
            .into_iter()
            .map(|s| s.label)
            .collect()
    }

    #[test]
    fn suggests_commands_with_specs() {
        let found = labels("gi");
        assert!(found.contains(&"git".to_string()), "{found:?}");
        assert!(labels("").is_empty());
    }

    #[test]
    fn suggests_whole_commands_from_history() {
        let completer = Completer::new(Vec::new())
            .with_history(History::new(["docker-compose up -d --build".to_string()]));
        let completion = completer.complete("docker-compose u", Path::new("/"));
        let first = &completion.suggestions[0];
        assert_eq!(first.kind, Kind::History);
        assert_eq!(first.label, "docker-compose up -d --build");
        assert_eq!(completion.replace, "u");
        assert_eq!(first.insert, "up -d --build");
        // Une commande exécutée devient la plus récente.
        completer.record("docker-compose up");
        let labels: Vec<String> = completer
            .complete("docker-compose ", Path::new("/"))
            .suggestions
            .into_iter()
            .map(|s| s.label)
            .collect();
        assert_eq!(
            labels,
            ["docker-compose up", "docker-compose up -d --build"]
        );
    }

    #[test]
    fn embeds_all_fig_specs() {
        let specs = spec::builtin();
        assert!(specs.len() > 700, "{}", specs.len());
        let symfony = specs
            .iter()
            .find(|s| s.names.contains(&"symfony".to_string()))
            .unwrap();
        // Lue seulement à la demande.
        assert!(symfony
            .command()
            .subcommands
            .iter()
            .any(|c| c.names.contains(&"server:start".to_string())));
    }

    #[test]
    fn suggests_only_installed_commands() {
        // `symfony` a une spec mais n'est pas installé ici.
        assert!(!labels("symfon").contains(&"symfony".to_string()));
    }

    #[test]
    fn suggests_subcommands() {
        let found = labels("git ch");
        assert!(found.contains(&"checkout".to_string()), "{found:?}");
        assert!(found.contains(&"cherry-pick".to_string()), "{found:?}");
        // Les noms qui commencent par « ch » d'abord, la recherche floue après.
        let prefixed = found.iter().take_while(|l| l.starts_with("ch")).count();
        assert!(prefixed >= 2, "{found:?}");
        assert!(
            found[prefixed..].iter().all(|l| !l.starts_with("ch")),
            "{found:?}"
        );
    }

    #[test]
    fn suggests_options_of_the_current_subcommand() {
        let found = labels("git commit --am");
        assert_eq!(found[0], "--amend");
        // Puis la recherche floue (« a » puis « m » : --allow-empty…).
        assert!(
            found[1..].iter().all(|l| !l.starts_with("--am")),
            "{found:?}"
        );
    }

    #[test]
    fn suggests_option_values() {
        let found = labels("git commit --cleanup ");
        assert!(found.contains(&"verbatim".to_string()), "{found:?}");
    }

    #[test]
    fn follows_nested_commands() {
        let found = labels("sudo git ch");
        assert!(found.contains(&"checkout".to_string()), "{found:?}");
        let found = labels("sudo gi");
        assert!(found.contains(&"git".to_string()), "{found:?}");
    }

    #[test]
    fn unknown_commands_give_nothing() {
        assert!(labels("commande-inconnue --").is_empty());
    }

    #[test]
    fn reports_the_word_to_replace() {
        let completion = completer().complete("git 'ch", Path::new("/"));
        assert_eq!(completion.replace, "'ch");
    }

    #[test]
    fn suggests_files_for_path_arguments() {
        let dir = std::env::temp_dir().join(format!("easytab-complete-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("mon dossier")).unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();

        let completion = completer().complete("cd m", &dir);
        let first = &completion.suggestions[0];
        assert_eq!(first.insert, r"mon\ dossier/");
        assert_eq!(first.kind, Kind::Folder);
        assert!(!first.append_space);

        let found: Vec<_> = completer()
            .complete("cat n", &dir)
            .suggestions
            .into_iter()
            .map(|s| s.insert)
            .collect();
        assert_eq!(found, ["notes.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn suggests_git_branches_from_generators() {
        use crate::Generators;
        use std::process::Command as Process;
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = std::env::temp_dir().join(format!("easytab-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let status = Process::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap()
                .status;
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        git(&["branch", "feature/login"]);

        let (ready, notified) = mpsc::channel();
        let completer = Completer::builtin().with_generators(Generators::start(move || {
            let _ = ready.send(());
        }));
        let mut completion = completer.complete("git switch fe", &dir);
        assert!(completion.pending);
        while completion.pending {
            notified.recv_timeout(Duration::from_secs(10)).unwrap();
            completion = completer.complete("git switch fe", &dir);
        }
        let found: Vec<_> = completion
            .suggestions
            .iter()
            .map(|s| s.insert.as_str())
            .collect();
        assert_eq!(found, ["feature/login"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_fuzzy_matches_and_ranks_by_usage() {
        let found = labels("git chk");
        assert_eq!(
            found.first().map(String::as_str),
            Some("checkout"),
            "{found:?}"
        );

        let completer = Completer::builtin();
        completer.record("git cherry-pick abc");
        completer.record("git cherry-pick def");
        let found: Vec<_> = completer
            .complete("git che", Path::new("/nonexistent"))
            .suggestions
            .into_iter()
            // Les commandes notées reviennent aussi par l'historique, en tête.
            .filter(|s| s.kind != Kind::History)
            .map(|s| s.label)
            .collect();
        assert_eq!(found[0], "cherry-pick", "{found:?}");
    }
}
