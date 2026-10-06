//! Moteur de suggestions : parcourt la spec de la commande en suivant les mots
//! déjà tapés, puis propose ce qui peut venir à la place du mot en cours.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::exec;
use crate::files;
use crate::generators::{Generated, Generators, Item};
use crate::help::HelpSpecs;
use crate::history::History;
use crate::line::{self, Token};
use crate::project;
use crate::pwsh::PowerShell;
use crate::rank::{self, best_match, Usage, LOOSE_RANK};
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
    /// Variable d'environnement (`$HOME`, `$env:PATH`).
    Variable,
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
    /// Les commandes sans spec sont décrites par leur `--help`.
    help: Option<HelpSpecs>,
    /// Les specs à partir de cet indice sont celles de l'utilisateur.
    custom_from: usize,
    by_name: HashMap<String, usize>,
    /// Specs chargées par `loadSpec` (`php/bin-console`), par chemin : lues à la
    /// première utilisation (voir [`spec::loadable`]).
    loadable: OnceLock<HashMap<String, Spec>>,
    builtin_loadable: bool,
    generators: Generators,
    usage: Mutex<Usage>,
    history: Mutex<History>,
    /// Dossier de l'utilisateur (`~/.ssh/config`…).
    home: Option<PathBuf>,
    /// Alias du shell (bash, zsh), par nom.
    aliases: HashMap<String, String>,
    /// Variables d'environnement reçues au démarrage, triées par nom.
    variables: Vec<(String, String)>,
}

/// Ce que les generators ont besoin de savoir sur la ligne.
struct Context<'a> {
    cwd: &'a Path,
    /// Mots de la commande, du nom de la commande au mot en cours compris.
    tokens: Vec<String>,
    /// Module JS de la spec en cours.
    module: Option<String>,
    /// La commande puis les sous-commandes suivies (`["composer", "run-script"]`).
    path: Vec<String>,
    /// Des generators calculent encore des suggestions.
    pending: bool,
    /// Un `generateSpec` calcule encore une partie de la spec.
    generating: bool,
}

/// Au plus autant de `loadSpec` à la suite (protège des boucles).
const MAX_LOADS: usize = 8;

/// État du parcours des mots, qui passe d'une sous-commande à l'autre.
struct Walk<'w, 'a> {
    /// Mot en cours.
    current: &'w Token,
    /// Options des commandes parentes qui restent valables.
    persistent: Vec<&'a Opt>,
    /// `--` a été tapé : les mots suivants ne sont plus des options.
    options_done: bool,
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
            help: None,
            custom_from: usize::MAX,
            by_name,
            loadable: OnceLock::new(),
            builtin_loadable: false,
            generators: Generators::disabled(),
            usage: Mutex::default(),
            history: Mutex::default(),
            home: std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from),
            aliases: HashMap::new(),
            variables: {
                let mut variables: Vec<(String, String)> = std::env::vars().collect();
                variables.sort();
                variables
            },
        }
    }

    /// Alias du shell : `g push` se complète comme `git push`.
    pub fn set_aliases(&mut self, aliases: HashMap<String, String>) {
        self.aliases = aliases;
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

    /// Suite de la commande la plus récente de l'historique qui prolonge
    /// `input`, affichée en gris après le curseur.
    pub fn inline(&self, input: &str) -> Option<String> {
        let typed = input.trim_start();
        let history = self.history.lock().unwrap();
        let full = history.matches(input).into_iter().next()?;
        let rest = full.strip_prefix(typed)?;
        (!rest.is_empty() && !rest.contains('\n')).then(|| rest.to_string())
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
        Self::builtin_for(cfg!(windows))
    }

    /// Sous Windows, les specs de ses outils passent avant celles de Fig :
    /// celles de `ping` et `where` y décrivent les versions Unix.
    fn builtin_for(windows: bool) -> Self {
        let mut specs = if windows { spec::windows() } else { Vec::new() };
        specs.extend(spec::builtin());
        let mut completer = Self::from_specs(specs);
        completer.installed = Some(OnceLock::new());
        completer.builtin_loadable = true;
        completer
    }

    /// Complète aussi les commandes PowerShell (`Remove-Item -Recurse`).
    pub fn with_powershell(mut self, powershell: PowerShell) -> Self {
        self.powershell = Some(powershell);
        self
    }

    /// Ajoute les specs de l'utilisateur (`~/.easytab/specs`) : elles passent
    /// avant les specs embarquées de même nom, et sont proposées même si la
    /// commande n'est pas dans le PATH (fonction ou alias du shell).
    pub fn with_custom(mut self, specs: Vec<Spec>) -> Self {
        self.custom_from = self.commands.len();
        for spec in specs {
            for name in &spec.names {
                self.by_name.insert(name.clone(), self.commands.len());
            }
            self.commands.push(spec);
        }
        self
    }

    /// Lit les options des commandes sans spec dans leur `--help`.
    pub fn with_help(mut self, help: HelpSpecs) -> Self {
        self.help = Some(help);
        self
    }

    /// Specs que les autres chargent par `loadSpec`, par chemin.
    pub fn with_loadable(self, loadable: HashMap<String, Command>) -> Self {
        let loadable = loadable
            .into_iter()
            .map(|(path, command)| (path, Spec::from(command)))
            .collect();
        let _ = self.loadable.set(loadable);
        self
    }

    /// Prend `home` comme dossier de l'utilisateur.
    #[cfg(test)]
    fn with_home(mut self, home: PathBuf) -> Self {
        self.home = Some(home);
        self
    }

    /// Prend `variables` comme variables d'environnement.
    #[cfg(test)]
    fn with_variables(mut self, variables: &[(&str, &str)]) -> Self {
        self.variables = variables
            .iter()
            .map(|&(name, value)| (name.to_string(), value.to_string()))
            .collect();
        self
    }

    /// Active les suggestions dynamiques.
    pub fn with_generators(mut self, generators: Generators) -> Self {
        self.generators = generators;
        self
    }

    pub fn complete(&self, input: &str, cwd: &Path) -> Completion {
        let line = line::parse(input);
        let powershell = self.powershell.is_some();
        if let Some(suggestions) = complete_variable(&line.current.raw, powershell, &self.variables)
        {
            return Completion {
                replace: line.current.raw,
                suggestions,
                pending: false,
            };
        }
        let words = self.expand_aliases(&line.words);
        let mut out = Vec::new();
        let mut context = Context {
            cwd,
            tokens: Vec::new(),
            module: None,
            path: Vec::new(),
            pending: false,
            generating: false,
        };
        self.complete_words(&words, &line.current, &mut context, &mut out);
        self.rank(&context.tokens, &mut out);
        drop_loose_matches(&mut out);
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
            pending: context.pending || context.generating,
        }
    }

    /// Remplace un alias en tête de ligne par sa valeur, comme le shell
    /// (`g push` → `git push`), plusieurs fois au besoin (`gs` → `g status`
    /// → `git status`). Un nom échappé ou entre guillemets (`\ls`) n'est pas
    /// un alias.
    fn expand_aliases(&self, words: &[Token]) -> Vec<Token> {
        let mut words = words.to_vec();
        let mut seen = HashSet::new();
        while let Some(first) = words.first() {
            let Some(value) = self.aliases.get(&first.value) else {
                break;
            };
            if first.raw != first.value || !seen.insert(first.value.clone()) {
                break;
            }
            let expansion = line::parse(&format!("{value} "));
            if expansion.words.is_empty() {
                break;
            }
            words.splice(0..1, expansion.words);
        }
        words
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

    /// La commande a une spec de l'utilisateur (`~/.easytab/specs`).
    fn is_custom(&self, name: &str) -> bool {
        self.by_name
            .get(name)
            .is_some_and(|&i| i >= self.custom_from)
    }

    fn find(&self, name: &str) -> Option<&Command> {
        // `/usr/bin/git` -> `git`
        let name = name.rsplit('/').next().unwrap_or(name);
        self.by_name.get(name).map(|&i| self.commands[i].command())
    }

    /// Spec désignée par un `loadSpec` : chargeable (`php/bin-console`) ou de
    /// premier niveau (`git`).
    fn find_loaded(&self, path: &str) -> Option<&Command> {
        let loadable = self.loadable.get_or_init(|| {
            if self.builtin_loadable {
                spec::loadable()
            } else {
                HashMap::new()
            }
        });
        match loadable.get(path) {
            Some(spec) => Some(spec.command()),
            None => self.by_name.get(path).map(|&i| self.commands[i].command()),
        }
    }

    fn complete_words(
        &self,
        words: &[Token],
        current: &Token,
        context: &mut Context,
        out: &mut Vec<Suggestion>,
    ) {
        let Some((name, rest)) = words.split_first() else {
            self.push_commands(&current.value, out);
            return;
        };
        // Une spec de l'utilisateur ou embarquée, sinon PowerShell, sinon `--help`.
        let alias = self
            .powershell
            .as_ref()
            .filter(|_| !self.is_custom(&name.value))
            .and_then(|powershell| Some((powershell, powershell.alias_target(&name.value)?)));
        let spec = match alias {
            // PowerShell résout un alias avant de chercher un programme : `ls`
            // y est `Get-ChildItem`, pas le `ls` d'Unix. Un alias vers un
            // programme (`g` → `git`) reçoit sa spec.
            Some((powershell, target)) => {
                self.find(&target).or_else(|| powershell.spec(&name.value))
            }
            None => self
                .find(&name.value)
                .or_else(|| {
                    self.powershell
                        .as_ref()
                        .and_then(|powershell| powershell.spec(&name.value))
                })
                .or_else(|| self.help.as_ref().and_then(|help| help.spec(&name.value))),
        };
        let Some(spec) = spec else {
            return;
        };
        context.module = spec.module.clone();
        context.path = spec.names.first().cloned().into_iter().collect();
        context.tokens = words
            .iter()
            .chain([current])
            .map(|t| t.value.clone())
            .collect();
        let walk = Walk {
            current,
            persistent: Vec::new(),
            options_done: false,
        };
        self.enter(spec, rest, walk, context, out);
    }

    /// Entre dans `node` : suit son `loadSpec`, ajoute ce que calcule son
    /// `generateSpec`, puis complète les mots `rest` qui suivent.
    fn enter<'a>(
        &'a self,
        mut node: &'a Command,
        rest: &[Token],
        walk: Walk<'_, 'a>,
        context: &mut Context,
        out: &mut Vec<Suggestion>,
    ) {
        for _ in 0..MAX_LOADS {
            let Some(loaded) = node.load.as_deref().and_then(|path| self.find_loaded(path)) else {
                break;
            };
            node = loaded;
            context.module = node.module.clone();
        }
        if let (Some(path), Some(module)) = (&node.generate, &context.module) {
            match self
                .generators
                .spec(module, path, node, context.cwd, &context.tokens)
            {
                Generated::Ready(Some(generated)) => {
                    return self.walk(&generated, rest, walk, context, out);
                }
                Generated::Ready(None) => {}
                Generated::Pending => context.generating = true,
            }
        }
        self.walk(node, rest, walk, context, out);
    }

    fn walk<'a>(
        &'a self,
        node: &'a Command,
        rest: &[Token],
        mut walk: Walk<'_, 'a>,
        context: &mut Context,
        out: &mut Vec<Suggestion>,
    ) {
        walk.persistent
            .extend(node.options.iter().filter(|o| o.persistent));
        let mut positional = 0;
        let mut seen_positional = false;
        // Mots qui ne sont ni des options ni leurs valeurs.
        let mut positionals: Vec<&str> = Vec::new();
        let mut pending: Option<&Arg> = None;

        for (i, word) in rest.iter().enumerate() {
            let word = word.value.as_str();
            if pending.take().is_some() {
                continue;
            }
            if !walk.options_done && word == "--" {
                walk.options_done = true;
                continue;
            }
            if !walk.options_done && word.len() > 1 && word.starts_with('-') {
                let (flag, has_value) = match word.split_once('=') {
                    Some((flag, _)) => (flag, true),
                    None => (word, false),
                };
                if let Some(opt) = find_option(node, &walk.persistent, flag) {
                    if !has_value {
                        pending = opt.args.first().filter(|arg| !arg.optional);
                    }
                }
                continue;
            }
            // Option à la Windows (`/MIR`, `/LOG:fichier`) ; un mot en `/` qui
            // n'en est pas une reste un chemin (Git Bash).
            if !walk.options_done {
                if let Some((opt, glued)) = find_slash_option(node, &walk.persistent, word) {
                    if !glued {
                        pending = opt.args.first().filter(|arg| !arg.optional);
                    }
                    continue;
                }
            }
            if !seen_positional {
                if let Some(sub) = find_subcommand(node, word) {
                    context.path.extend(sub.names.first().cloned());
                    return self.enter(sub, &rest[i + 1..], walk, context, out);
                }
            }
            seen_positional = true;
            positionals.push(word);
            if let Some(arg) = node.args.get(positional) {
                if arg.is_command {
                    // `sudo git ch` : on recommence avec la commande imbriquée.
                    return self.complete_words(&rest[i..], walk.current, context, out);
                }
                // `loadSpec` d'un argument : la suite vient d'une autre spec.
                if let Some(spec) = arg.spec.as_deref() {
                    return self.enter(spec, &rest[i + 1..], walk, context, out);
                }
                if let Some(loaded) = arg.load.as_deref().and_then(|p| self.find_loaded(p)) {
                    context.module = loaded.module.clone();
                    return self.enter(loaded, &rest[i + 1..], walk, context, out);
                }
                if !arg.variadic {
                    positional += 1;
                }
            }
        }

        let persistent = &walk.persistent;
        let prefix = walk.current.value.as_str();
        if let Some(arg) = pending {
            self.push_arg(arg, prefix, "", context, out);
            return;
        }
        if !walk.options_done && prefix.starts_with('-') {
            if let Some((flag, value)) = prefix.split_once('=') {
                if let Some(arg) = find_option(node, persistent, flag).and_then(|o| o.args.first())
                {
                    self.push_arg(arg, value, &format!("{flag}="), context, out);
                }
            } else {
                push_options(node, persistent, prefix, out);
            }
            return;
        }
        if !walk.options_done && prefix.starts_with('/') && has_slash_options(node, persistent) {
            // Valeur collée à l'option : `/LOG:jou` complète un fichier.
            if let Some((opt, true)) = find_slash_option(node, persistent, prefix) {
                let name = opt
                    .names
                    .iter()
                    .find(|n| glued_value(n, prefix).is_some())
                    .map_or(0, String::len);
                if let Some(arg) = opt.args.first() {
                    self.push_arg(arg, &prefix[name..], &prefix[..name], context, out);
                }
                return;
            }
            let before = out.len();
            push_options(node, persistent, prefix, out);
            if out.len() > before {
                return;
            }
        }
        if !seen_positional {
            push_subcommands(node, prefix, out);
        }
        // Valeurs lues dans les fichiers du projet (cibles make…), avant celles
        // des generators : les doublons de ces derniers sont écartés.
        let source = project::source(&context.path, &positionals);
        let native = out.len()..;
        if let Some(source) = source {
            let items = project::items(source, context.cwd, self.home.as_deref(), &context.tokens);
            let generator = Generator {
                query_term: source.query_term().map(str::to_string),
                ..Generator::default()
            };
            push_generated(&items, &generator, prefix, "", out);
        }
        let native = native.start..out.len();
        if let Some(arg) = node.args.get(positional) {
            if arg.is_command {
                self.push_commands(prefix, out);
            } else {
                self.push_arg(arg, prefix, "", context, out);
            }
        }
        if let Some(project::Source::SshHosts { .. }) = source {
            drop_host_duplicates(out, native);
        }
        // Les options ne servent qu'à défaut d'autre chose : pas pendant que des
        // generators calculent.
        if out.is_empty() && prefix.is_empty() && !context.pending {
            push_options(node, persistent, prefix, out);
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
        // Dans PowerShell, `ls` est l'alias de Get-ChildItem : il est proposé
        // comme tel, pas avec la description du `ls` d'Unix.
        let aliases = self
            .powershell
            .as_ref()
            .map(PowerShell::aliases)
            .unwrap_or_default();
        let mut found: Vec<Suggestion> = self
            .commands
            .iter()
            .enumerate()
            .flat_map(|(i, c)| c.names.iter().map(move |n| (i, n, c)))
            // Une spec embarquée remplacée par celle de l'utilisateur n'est pas
            // proposée.
            .filter(|(i, name, _)| self.by_name.get(name.as_str()) == Some(i))
            .filter(|(i, name, _)| {
                *i >= self.custom_from
                    || (installed.is_none_or(|set| set.contains(name.as_str()))
                        && !aliases.contains(&name.to_lowercase()))
            })
            .filter_map(|(_, name, command)| {
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
            found.extend(
                powershell
                    .commands()
                    .into_iter()
                    .filter_map(|(name, target)| {
                        Some(Suggestion {
                            rank: rank::match_rank(&name, prefix)?,
                            insert: name.clone(),
                            label: name,
                            // Un alias : la commande qu'il désigne.
                            description: target,
                            kind: Kind::Command,
                            hint: None,
                            icon: None,
                            append_space: true,
                        })
                    }),
            );
        }
        found.extend(self.aliases.iter().filter_map(|(name, value)| {
            Some(Suggestion {
                rank: rank::match_rank(name, prefix)?,
                label: name.clone(),
                insert: name.clone(),
                description: Some(value.clone()),
                kind: Kind::Command,
                hint: None,
                icon: None,
                append_space: true,
            })
        }));
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

        for generator in &arg.generators {
            // Le generator d'une spec produite par `generateSpec` a son module.
            if let Some(module) = generator.module.as_deref().or(context.module.as_deref()) {
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

fn has_slash_options(node: &Command, persistent: &[&Opt]) -> bool {
    node.options
        .iter()
        .chain(persistent.iter().copied())
        .any(|o| o.names.iter().any(|n| n.starts_with('/')))
}

/// Option à la Windows désignée par `word`, sans tenir compte de la casse
/// (`/mir` vaut `/MIR`), et si sa valeur y est collée (`/LOG:fichier`,
/// `/scanfile=fichier` : l'option est nommée `/LOG:`, `/scanfile=`).
fn find_slash_option<'a>(
    node: &'a Command,
    persistent: &[&'a Opt],
    word: &str,
) -> Option<(&'a Opt, bool)> {
    if !word.starts_with('/') {
        return None;
    }
    let options = || node.options.iter().chain(persistent.iter().copied());
    let names = |o: &'a Opt| o.names.iter().filter(|n| n.starts_with('/'));
    options()
        .find_map(|o| {
            names(o)
                .find(|n| n.eq_ignore_ascii_case(word))
                .map(|n| (o, n.ends_with([':', '='])))
        })
        .or_else(|| {
            options().find_map(|o| names(o).find_map(|n| glued_value(n, word).map(|_| (o, true))))
        })
}

/// Valeur collée à l'option `name` (`/LOG:`) dans `word` (`/log:a.txt` → `a.txt`).
fn glued_value<'w>(name: &str, word: &'w str) -> Option<&'w str> {
    if !name.ends_with([':', '=']) {
        return None;
    }
    let head = word.get(..name.len())?;
    head.eq_ignore_ascii_case(name).then(|| &word[name.len()..])
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
            // `/LOG:` : la valeur se colle à l'option.
            let append_space = !takes_equals && !insert.ends_with([':', '=']);
            Some(Suggestion {
                rank,
                label: o.names.join(", "),
                insert,
                description: o.description.clone(),
                hint: args_hint(&o.args),
                icon: None,
                kind: Kind::Option,
                append_space,
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

/// Hôtes SSH : `native` (lus dans `~/.ssh`) passent avant ceux des
/// generators Fig, qui sont écartés s'ils insèrent la même chose (`marc@hôte`
/// sous une autre étiquette) ou la même chose sans `:` (`scp`). Après un `:`,
/// pas d'espace : le chemin suit.
fn drop_host_duplicates(out: &mut Vec<Suggestion>, native: std::ops::Range<usize>) {
    let inserted: HashSet<String> = out[native.clone()]
        .iter()
        .map(|s| s.insert.clone())
        .collect();
    for suggestion in &mut out[native.clone()] {
        if suggestion.insert.ends_with(':') {
            suggestion.append_space = false;
        }
    }
    let mut index = 0;
    out.retain(|s| {
        let duplicate = index >= native.end
            && s.kind == Kind::Dynamic
            && (inserted.contains(&s.insert) || inserted.contains(&format!("{}:", s.insert)));
        index += 1;
        !duplicate
    });
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

/// Complète une variable d'environnement : `$HO` → `$HOME`, `${HO` →
/// `${HOME}` ; sous PowerShell, `$env:PA` → `$env:PATH`. `None` si le mot
/// ne se termine pas par une variable.
fn complete_variable(
    raw: &str,
    powershell: bool,
    variables: &[(String, String)],
) -> Option<Vec<Suggestion>> {
    let dollar = raw.rfind('$')?;
    if raw[..dollar].ends_with('\\') {
        return None;
    }
    let after = &raw[dollar + 1..];
    let (head, close) = if powershell {
        if !after.get(..4)?.eq_ignore_ascii_case("env:") {
            return None;
        }
        (4, "")
    } else if after.starts_with('{') {
        (1, "}")
    } else {
        (0, "")
    };
    let typed = &after[head..];
    let is_name = |name: &str| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !is_name(typed) {
        return None;
    }
    let sigil = &raw[dollar..dollar + 1 + head];
    let before = &raw[..dollar];
    let mut found: Vec<Suggestion> = variables
        .iter()
        .filter(|(name, _)| !name.is_empty() && name != "_" && is_name(name))
        .filter_map(|(name, value)| {
            Some(Suggestion {
                rank: rank::match_rank(name, typed)?,
                label: format!("{sigil}{name}"),
                insert: format!("{before}{sigil}{name}{close}"),
                // Valeurs qui changent pendant la session : celle du
                // démarrage induirait en erreur.
                description: (!matches!(name.as_str(), "PWD" | "OLDPWD" | "SHLVL"))
                    .then(|| shorten(value, 120)),
                hint: None,
                icon: None,
                kind: Kind::Variable,
                append_space: false,
            })
        })
        .collect();
    found.sort_by(|a, b| (a.rank, &a.label).cmp(&(b.rank, &b.label)));
    drop_loose_matches(&mut found);
    Some(found)
}

/// Coupe `text` à `max` caractères, avec « … » s'il est plus long.
fn shorten(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

fn sort(found: &mut [Suggestion]) {
    found.sort_by(|a, b| a.label.cmp(&b.label));
}

/// Écarte les correspondances floues (lettres dans l'ordre, rang 3) quand
/// d'autres suggestions commencent par le mot tapé ou le contiennent : `git che`
/// propose `checkout` et `cherry-pick`, pas `credential-helper-selector`.
/// Sans meilleure correspondance, elles restent (`git chk` → `checkout`).
fn drop_loose_matches(out: &mut Vec<Suggestion>) {
    if out.iter().any(|s| s.rank < LOOSE_RANK) {
        out.retain(|s| s.rank < LOOSE_RANK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let completer = Completer::new(Vec::new()).with_history(History::new([
            "docker-compose down".to_string(),
            "docker-compose up -d --build".to_string(),
        ]));
        let completion = completer.complete("docker-compose u", Path::new("/"));
        assert_eq!(
            completion.suggestions.len(),
            1,
            "{:?}",
            completion.suggestions
        );
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
            [
                "docker-compose up",
                "docker-compose up -d --build",
                "docker-compose down"
            ]
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
        // Seul `git` est installé : `symfony` a une spec mais n'est pas proposé.
        let mut completer = Completer::builtin();
        completer.installed = Some(OnceLock::from(HashSet::from(["git".to_string()])));
        let labels = |input: &str| -> Vec<String> {
            completer
                .complete(input, &PathBuf::from("/nonexistent"))
                .suggestions
                .into_iter()
                .map(|s| s.label)
                .collect()
        };
        assert!(!labels("symfon").contains(&"symfony".to_string()));
        assert!(labels("gi").contains(&"git".to_string()));
    }

    /// Completer avec les specs de Windows, où seuls `installed` sont dans le PATH.
    fn windows_completer(installed: &[&str]) -> Completer {
        let mut completer = Completer::builtin_for(true);
        completer.installed = Some(OnceLock::from(
            installed
                .iter()
                .map(|s| s.to_string())
                .collect::<HashSet<_>>(),
        ));
        completer
    }

    fn suggestions(completer: &Completer, input: &str) -> Vec<Suggestion> {
        completer
            .complete(input, &PathBuf::from("/nonexistent"))
            .suggestions
    }

    #[test]
    fn embeds_windows_specs() {
        let specs = spec::windows();
        let names: Vec<&str> = specs.iter().map(|s| s.names[0].as_str()).collect();
        for tool in [
            "winget",
            "wsl",
            "choco",
            "scoop",
            "ipconfig",
            "netsh",
            "robocopy",
            "taskkill",
            "tasklist",
            "sc",
            "dism",
            "sfc",
            "where",
            "findstr",
            "xcopy",
            "icacls",
            "schtasks",
            "shutdown",
            "systeminfo",
            "nslookup",
            "ping",
            "tracert",
            "net",
        ] {
            assert!(names.contains(&tool), "{tool} manque : {names:?}");
        }
        // Lues en entier, sans erreur, avec une description pour chaque commande.
        for spec in &specs {
            assert!(spec.description.is_some(), "{:?}", spec.names);
            let command = spec.command();
            assert!(
                !command.options.is_empty() || !command.subcommands.is_empty(),
                "{:?}",
                spec.names
            );
        }
    }

    #[test]
    fn windows_specs_only_on_windows_and_when_installed() {
        let windows = windows_completer(&["ping", "robocopy"]);
        let ping = suggestions(&windows, "pin");
        let ping = ping.iter().find(|s| s.label == "ping").unwrap();
        assert_eq!(
            ping.description.as_deref(),
            Some("Send ICMP echo requests to network hosts")
        );
        assert!(labels_of(&windows, "ping -").contains(&"-n".to_string()));
        assert!(labels_of(&windows, "rob").contains(&"robocopy".to_string()));
        // `winget` n'est pas installé : pas proposé.
        assert!(!labels_of(&windows, "wing").contains(&"winget".to_string()));

        // Ailleurs, `ping` garde la spec Fig, et `robocopy` n'existe pas.
        let mut unix = Completer::builtin_for(false);
        unix.installed = Some(OnceLock::from(HashSet::from([
            "ping".to_string(),
            "robocopy".to_string(),
        ])));
        let ping = suggestions(&unix, "pin");
        let ping = ping.iter().find(|s| s.label == "ping").unwrap();
        assert_ne!(
            ping.description.as_deref(),
            Some("Send ICMP echo requests to network hosts")
        );
        assert!(!labels_of(&unix, "rob").contains(&"robocopy".to_string()));
    }

    #[test]
    fn completes_windows_subcommands_and_options() {
        let completer = windows_completer(&[]);
        assert!(labels_of(&completer, "winget ins").contains(&"install".to_string()));
        assert!(labels_of(&completer, "winget install --sc").contains(&"--scope".to_string()));
        assert_eq!(
            labels_of(&completer, "winget install --scope "),
            ["machine", "user"]
        );
        assert!(labels_of(&completer, "netsh wlan show pro").contains(&"profiles".to_string()));
        assert!(labels_of(&completer, "sc qu").contains(&"query".to_string()));
        assert!(labels_of(&completer, "wsl --ins").contains(&"--install".to_string()));
    }

    #[test]
    fn completes_slash_options() {
        let completer = windows_completer(&[]);
        let found = suggestions(&completer, "robocopy src dst /MI");
        let mir = found.iter().find(|s| s.label == "/MIR").expect("/MIR");
        assert!(mir.append_space);
        // Sans tenir compte de la casse, et après d'autres options.
        assert!(labels_of(&completer, "robocopy src dst /mir /pu").contains(&"/PURGE".to_string()));
        // La valeur d'une option se colle après `:` : pas d'espace.
        let found = suggestions(&completer, "robocopy src dst /LO");
        let log = found.iter().find(|s| s.label == "/LOG:").expect("/LOG:");
        assert_eq!(log.insert, "/LOG:");
        assert!(!log.append_space);
        // Valeur collée : seule la fin du mot est complétée, la casse tapée gardée.
        let found = suggestions(&completer, "dism /online /enable-feature /featurename:Virt");
        assert_eq!(found[0].label, "VirtualMachinePlatform");
        assert_eq!(found[0].insert, "/featurename:VirtualMachinePlatform");
        let found = suggestions(&completer, "robocopy a b /COPY:DATS");
        assert_eq!(found[0].label, "DATS");
        // Valeur dans le mot suivant.
        assert_eq!(
            labels_of(&completer, "tasklist /fo "),
            ["CSV", "LIST", "TABLE"]
        );
        // `/scanfile=` : la valeur suit le `=`.
        let found = suggestions(&completer, "sfc /scan");
        let scanfile = found.iter().find(|s| s.label == "/scanfile=").unwrap();
        assert!(!scanfile.append_space);
        // Une commande sans option en `/` laisse les chemins tranquilles.
        assert!(labels_of(&completer, "winget /").is_empty());
    }

    #[test]
    fn powershell_aliases_get_their_command_parameters() {
        let powershell = PowerShell::answered(
            r#"[{"n":"Get-ChildItem","t":"Cmdlet","r":null},
                {"n":"ls","t":"Alias","r":"Get-ChildItem"},
                {"n":"gci","t":"Alias","r":"Get-ChildItem"},
                {"n":"g","t":"Alias","r":"git"},
                {"n":"%","t":"Alias","r":"ForEach-Object"}]"#,
            &[(
                "Get-ChildItem",
                r#"[{"n":"Path","a":[],"s":false,"t":"String[]","v":[]},
                    {"n":"Recurse","a":["s"],"s":true,"t":"SwitchParameter","v":[]}]"#,
            )],
        );
        let completer = Completer::new(vec![
            serde_json::from_str(
                r#"{"names": ["ls"], "description": "List directory contents",
                    "options": [{"names": ["-l"]}]}"#,
            )
            .unwrap(),
            serde_json::from_str(r#"{"names": ["git"], "subcommands": [{"names": ["checkout"]}]}"#)
                .unwrap(),
        ])
        .with_powershell(powershell);

        // `ls` est Get-ChildItem, pas le `ls` d'Unix.
        let found = labels_of(&completer, "ls -");
        assert!(found.contains(&"-Recurse, -s".to_string()), "{found:?}");
        assert!(!found.contains(&"-l".to_string()), "{found:?}");
        assert!(labels_of(&completer, "gci -Pa").contains(&"-Path".to_string()));
        // Un alias vers un programme reçoit la spec de celui-ci.
        assert!(labels_of(&completer, "g che").contains(&"checkout".to_string()));

        // Dans la liste des commandes, avec la commande désignée.
        let found = suggestions(&completer, "l");
        let ls: Vec<_> = found.iter().filter(|s| s.label == "ls").collect();
        assert_eq!(ls.len(), 1, "{found:?}");
        assert_eq!(ls[0].description.as_deref(), Some("Get-ChildItem"));
        let found = suggestions(&completer, "gc");
        let gci = found.iter().find(|s| s.label == "gci").unwrap();
        assert_eq!(gci.description.as_deref(), Some("Get-ChildItem"));
        assert!(labels_of(&completer, "%").is_empty());
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

    fn variables() -> Vec<(String, String)> {
        [
            ("HOME", "/home/adam"),
            ("HOSTNAME", "pc"),
            ("PATH", "/usr/bin"),
            ("PWD", "/tmp"),
        ]
        .iter()
        .map(|&(n, v)| (n.to_string(), v.to_string()))
        .collect()
    }

    fn inserts(found: Option<Vec<Suggestion>>) -> Vec<String> {
        found.unwrap().into_iter().map(|s| s.insert).collect()
    }

    #[test]
    fn completes_environment_variables() {
        let vars = variables();
        assert_eq!(
            inserts(complete_variable("$HO", false, &vars)),
            ["$HOME", "$HOSTNAME"]
        );
        assert_eq!(
            inserts(complete_variable("\"$HO", false, &vars)),
            ["\"$HOME", "\"$HOSTNAME"]
        );
        assert_eq!(
            inserts(complete_variable("${PA", false, &vars)),
            ["${PATH}"]
        );
        assert_eq!(
            inserts(complete_variable("$env:pa", true, &vars)),
            ["$env:PATH"]
        );
        let home = complete_variable("$HOM", false, &vars).unwrap();
        assert_eq!(home[0].description.as_deref(), Some("/home/adam"));
        assert_eq!(home[0].label, "$HOME");
        assert!(!home[0].append_space);
        let pwd = complete_variable("$PW", false, &vars).unwrap();
        assert_eq!(pwd[0].description, None);
        // Pas une variable : complétion habituelle.
        assert!(complete_variable("$HO", true, &vars).is_none());
        assert!(complete_variable("\\$HO", false, &vars).is_none());
        assert!(complete_variable("$?", false, &vars).is_none());
        assert!(complete_variable("src/", false, &vars).is_none());

        let completer = Completer::builtin().with_variables(&[("HOME", "/home/adam")]);
        let found = completer.complete("cd $HO", Path::new("/nonexistent"));
        assert_eq!(found.replace, "$HO");
        assert_eq!(found.suggestions[0].kind, Kind::Variable);
    }

    #[test]
    fn follows_shell_aliases() {
        let mut completer = Completer::new(vec![command(
            r#"{"names": ["git"], "subcommands": [{"names": ["push"]},
                {"names": ["status"], "options": [{"names": ["--short"]}]}]}"#,
        )]);
        completer.set_aliases(HashMap::from([
            ("g".to_string(), "git".to_string()),
            ("gs".to_string(), "g status".to_string()),
            ("git".to_string(), "git".to_string()),
        ]));
        assert_eq!(labels_of(&completer, "g pu"), ["push"]);
        assert_eq!(labels_of(&completer, "gs --sh"), ["--short"]);
        assert_eq!(labels_of(&completer, "git pu"), ["push"]);
        // `\g` contourne l'alias.
        assert!(labels_of(&completer, "\\g pu").is_empty());
        // Les alias sont proposés comme commandes, avec leur valeur.
        let found = completer.complete("gs", Path::new("/nonexistent"));
        let alias = found.suggestions.iter().find(|s| s.label == "gs").unwrap();
        assert_eq!(alias.description.as_deref(), Some("g status"));
    }

    #[test]
    fn hides_loose_matches_behind_better_ones() {
        let completer = Completer::new(vec![command(
            r#"{"names": ["g"], "subcommands": [{"names": ["checkout"]},
                {"names": ["credential-helper-selector"]}]}"#,
        )]);
        // `che` : `checkout` commence par le mot, la correspondance floue part.
        assert_eq!(labels_of(&completer, "g che"), ["checkout"]);
        // Sans meilleure correspondance, la recherche floue reste.
        assert_eq!(labels_of(&completer, "g chk"), ["checkout"]);
        assert_eq!(
            labels_of(&completer, "g chs"),
            ["credential-helper-selector"]
        );
    }

    fn command(json: &str) -> Command {
        serde_json::from_str(json).unwrap()
    }

    fn labels_of(completer: &Completer, input: &str) -> Vec<String> {
        completer
            .complete(input, Path::new("/"))
            .suggestions
            .into_iter()
            .map(|s| s.label)
            .collect()
    }

    #[test]
    fn follows_load_spec() {
        let completer = Completer::new(vec![command(
            r#"{"names": ["outil"], "subcommands": [{"names": ["sous"], "load": "outil/sous"}],
                "args": [{"name": "cible", "load": "outil/profond"}],
                "options": [{"names": ["--projet"], "args": [{"name": "projet",
                    "spec": {"names": ["projet"], "options": [{"names": ["--sur-place"]}]}}]}]}"#,
        )])
        .with_loadable(HashMap::from([
            (
                "outil/sous".to_string(),
                command(r#"{"names": ["sous"], "subcommands": [{"names": ["plus-loin"], "load": "outil/profond"}]}"#),
            ),
            (
                "outil/profond".to_string(),
                command(r#"{"names": ["profond"], "options": [{"names": ["--profond"]}]}"#),
            ),
        ]));
        assert_eq!(labels_of(&completer, "outil sous "), ["plus-loin"]);
        // Une spec chargée peut en charger une autre.
        assert_eq!(
            labels_of(&completer, "outil sous plus-loin --"),
            ["--profond"]
        );
        // `loadSpec` d'un argument : après lui, la suite vient de l'autre spec.
        assert_eq!(labels_of(&completer, "outil truc --"), ["--profond"]);
        // Les specs chargeables ne sont pas des commandes.
        assert!(labels_of(&completer, "outil/so").is_empty());
    }

    #[test]
    fn follows_builtin_load_specs() {
        // `dotnet build` vient de `dotnet/dotnet-build`.
        let found = labels("dotnet build --verb");
        assert!(found.contains(&"-v, --verbosity".to_string()), "{found:?}");
    }

    /// Complète `input` jusqu'à ce que generators et `generateSpec` aient fini.
    #[cfg(unix)]
    fn settle(
        completer: &Completer,
        notified: &std::sync::mpsc::Receiver<()>,
        input: &str,
        cwd: &Path,
    ) -> Completion {
        let mut completion = completer.complete(input, cwd);
        while completion.pending {
            notified
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            completion = completer.complete(input, cwd);
        }
        completion
    }

    #[cfg(unix)]
    #[test]
    fn merges_generate_spec_results() {
        use crate::generators::tests::GENERATED_MODULE;
        use std::sync::mpsc;

        let (ready, notified) = mpsc::channel();
        let completer = Completer::new(vec![command(
            r#"{"names": ["gen"], "module": "gen", "generate": [],
                "subcommands": [{"names": ["statique"]}, {"names": ["remplacee"]}]}"#,
        )])
        .with_generators(Generators::start_with(
            HashMap::from([("gen".to_string(), GENERATED_MODULE.to_string())]),
            move || {
                let _ = ready.send(());
            },
        ));
        // En attendant `generateSpec` : la spec statique, sans bloquer.
        let first = completer.complete("gen ", Path::new("/"));
        assert!(first.pending);
        let found: Vec<_> = first.suggestions.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(found, ["remplacee", "statique"]);

        let found = settle(&completer, &notified, "gen ", Path::new("/"));
        let found: Vec<_> = found.suggestions.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(found, ["dynamique", "remplacee", "statique"]);
        assert_eq!(labels_of(&completer, "gen --g"), ["--genere"]);
        // Le generator de la sous-commande générée (`gen/charge` n'existe pas :
        // la sous-commande garde son contenu). Il voit les mots qu'a reçus
        // `generateSpec` (deux).
        let found = settle(&completer, &notified, "gen dynamique ", Path::new("/"));
        let found: Vec<_> = found.suggestions.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(found, ["valeur-2"]);
    }

    /// `php` propose `bin/console` dans un projet Symfony (son `generateSpec`
    /// lance `ls bin/console`), qui se complète avec `php/bin-console`.
    #[cfg(unix)]
    #[test]
    fn php_suggests_bin_console_in_symfony_projects() {
        use std::sync::mpsc;

        let dir = std::env::temp_dir().join(format!("easytab-php-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/console"), "").unwrap();

        let (ready, notified) = mpsc::channel();
        let completer = Completer::builtin().with_generators(Generators::start(move || {
            let _ = ready.send(());
        }));
        let found = settle(&completer, &notified, "php ", &dir);
        let found: Vec<_> = found.suggestions.iter().map(|s| s.label.as_str()).collect();
        assert!(found.contains(&"bin/console"), "{found:?}");
        assert!(!found.contains(&"artisan"), "{found:?}");
        // `php/bin-console` lance à son tour `php bin/console list` : sans PHP
        // ici, il ne donne rien, mais le calcul se termine.
        settle(&completer, &notified, "php bin/console ", &dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("easytab-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn suggests_project_values() {
        let dir = temp_dir("projet");
        std::fs::write(
            dir.join("Makefile"),
            "build: ## Compile\n\tcc\ntest: build\n\tcc\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("composer.json"),
            r#"{"scripts": {"lint": "phpcs", "test": "phpunit"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("angular.json"),
            r#"{"projects": {"boutique": {"projectType": "application"}}}"#,
        )
        .unwrap();
        let found = |input: &str| -> Vec<String> {
            completer()
                .complete(input, &dir)
                .suggestions
                .into_iter()
                .map(|s| s.label)
                .collect()
        };
        assert_eq!(found("make "), ["build", "test"]);
        assert_eq!(found("make -k te"), ["test"]);
        let completion = completer().complete("make b", &dir);
        assert_eq!(completion.suggestions[0].kind, Kind::Dynamic);
        assert_eq!(
            completion.suggestions[0].description.as_deref(),
            Some("Compile")
        );
        assert_eq!(found("composer run-script "), ["lint", "test"]);
        assert_eq!(found("composer run l"), ["lint"]);
        assert_eq!(found("ng build "), ["boutique"]);
        assert!(!found("ng new ").contains(&"boutique".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Les cibles lues dans le Makefile et celles du generator de la spec Fig
    /// (`make -qp`) ne sont pas proposées deux fois.
    #[cfg(unix)]
    #[test]
    fn make_targets_are_not_duplicated_by_generators() {
        use std::sync::mpsc;

        let dir = temp_dir("make");
        std::fs::write(
            dir.join("Makefile"),
            "build:\n\ttrue\ntest: build\n\ttrue\n",
        )
        .unwrap();
        let (ready, notified) = mpsc::channel();
        let completer = Completer::builtin().with_generators(Generators::start(move || {
            let _ = ready.send(());
        }));
        // Tout de suite, avant le generator.
        let first = completer.complete("make ", &dir);
        let found: Vec<_> = first.suggestions.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(found, ["build", "test"]);
        let settled = settle(&completer, &notified, "make ", &dir);
        let found: Vec<_> = settled
            .suggestions
            .iter()
            .map(|s| s.label.as_str())
            .collect();
        let unique: HashSet<_> = found.iter().collect();
        assert_eq!(unique.len(), found.len(), "{found:?}");
        assert!(found.starts_with(&["build", "test"]), "{found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggests_package_scripts() {
        let dir = temp_dir("package");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts": {"build": "vite build", "test": "vitest"}}"#,
        )
        .unwrap();
        let src = dir.join("src");
        let found = |input: &str| -> Vec<String> {
            completer()
                .complete(input, &src)
                .suggestions
                .into_iter()
                .map(|s| s.label)
                .collect()
        };
        for input in [
            "npm run ",
            "npm run-script ",
            "yarn run ",
            "pnpm run ",
            "bun run ",
        ] {
            assert_eq!(found(input), ["build", "test"], "{input}");
        }
        // `yarn <script>`, `pnpm <script>` : avec les sous-commandes.
        assert!(found("yarn bui").contains(&"build".to_string()));
        assert!(found("pnpm ").contains(&"test".to_string()));
        let completion = completer().complete("npm run t", &src);
        assert_eq!(completion.suggestions[0].kind, Kind::Dynamic);
        assert_eq!(
            completion.suggestions[0].description.as_deref(),
            Some("vitest")
        );
        // Le script choisi, la suite n'en est plus un.
        assert!(!found("npm run build ").contains(&"test".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggests_ssh_hosts() {
        let home = temp_dir("ssh-hosts");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(
            home.join(".ssh/config"),
            "Host prod\n\tHostName 10.0.0.1\nHost *\n\tUser marc\n",
        )
        .unwrap();
        std::fs::write(
            home.join(".ssh/known_hosts"),
            "github.com ssh-ed25519 AAAA\n",
        )
        .unwrap();
        let completer = Completer::builtin().with_home(home.clone());
        let complete = |input: &str| completer.complete(input, &home).suggestions;
        let found =
            |input: &str| -> Vec<String> { complete(input).into_iter().map(|s| s.label).collect() };
        assert_eq!(found("ssh "), ["prod", "github.com"]);
        assert_eq!(found("sftp pr"), ["prod"]);
        assert_eq!(
            complete("ssh pr")[0].description.as_deref(),
            Some("10.0.0.1")
        );
        // Après `user@`, seul le nom d'hôte est complété.
        let found_at = complete("ssh marc@pr");
        assert_eq!(found_at[0].label, "prod");
        assert_eq!(found_at[0].insert, "marc@prod");
        // scp : `hôte:`, sans espace après, pour chaque argument.
        let scp = complete("scp fichier.txt pr");
        assert_eq!(scp[0].label, "prod:");
        assert_eq!(scp[0].insert, "prod:");
        assert!(!scp[0].append_space);
        assert!(!found("scp prod:").contains(&"prod:".to_string()));
        // Après l'hôte vient la commande.
        assert!(!found("ssh prod ").contains(&"github.com".to_string()));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn drops_ssh_hosts_of_generators_already_read() {
        let suggestion = |label: &str, insert: &str, kind: Kind| Suggestion {
            label: label.to_string(),
            insert: insert.to_string(),
            description: None,
            hint: None,
            icon: None,
            kind,
            append_space: true,
            rank: 0,
        };
        let mut out = vec![
            suggestion("-v", "-v", Kind::Option),
            suggestion("prod:", "prod:", Kind::Dynamic),
            suggestion("marc@gh", "marc@gh", Kind::Dynamic),
            // Ceux du generator Fig.
            suggestion("prod", "prod", Kind::Dynamic),
            suggestion("gh", "marc@gh", Kind::Dynamic),
            suggestion("autre", "autre", Kind::Dynamic),
            suggestion("prod", "prod", Kind::File),
        ];
        drop_host_duplicates(&mut out, 1..3);
        let labels: Vec<_> = out.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["-v", "prod:", "marc@gh", "autre", "prod"]);
        assert!(!out[1].append_space);
        assert!(out[2].append_space);
    }

    /// Les scripts lus dans package.json et ceux du generator de la spec Fig
    /// ne sont pas proposés deux fois.
    #[cfg(unix)]
    #[test]
    fn package_scripts_are_not_duplicated_by_generators() {
        use std::sync::mpsc;

        let dir = temp_dir("npm");
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts": {"build": "vite build", "test": "vitest"}}"#,
        )
        .unwrap();
        let (ready, notified) = mpsc::channel();
        let completer = Completer::builtin().with_generators(Generators::start(move || {
            let _ = ready.send(());
        }));
        for input in ["npm run ", "yarn run ", "pnpm run "] {
            let first = completer.complete(input, &dir);
            let found: Vec<_> = first.suggestions.iter().map(|s| s.label.as_str()).collect();
            assert!(found.starts_with(&["build", "test"]), "{input} {found:?}");
            let settled = settle(&completer, &notified, input, &dir);
            let found: Vec<_> = settled
                .suggestions
                .iter()
                .map(|s| s.label.as_str())
                .collect();
            let unique: HashSet<_> = found.iter().collect();
            assert_eq!(unique.len(), found.len(), "{input} {found:?}");
            assert!(found.starts_with(&["build", "test"]), "{input} {found:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_specs_replace_builtin_ones() {
        let custom = |json: &str| Spec::from(command(json));
        let completer = Completer::new(vec![
            command(r#"{"names": ["outil"], "subcommands": [{"names": ["integree"]}]}"#),
            command(r#"{"names": ["autre"], "subcommands": [{"names": ["garde"]}]}"#),
        ])
        .with_custom(vec![
            custom(r#"{"names": ["outil"], "description": "à moi", "subcommands": [{"names": ["perso"]}]}"#),
            custom(r#"{"names": ["nouveau"]}"#),
        ]);
        assert_eq!(labels_of(&completer, "outil "), ["perso"]);
        assert_eq!(labels_of(&completer, "autre "), ["garde"]);
        // Une seule fois dans les commandes, avec la description de l'utilisateur.
        let found = completer.complete("outi", Path::new("/")).suggestions;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].description.as_deref(), Some("à moi"));
        assert_eq!(labels_of(&completer, "nouv"), ["nouveau"]);
        // Les commandes de l'utilisateur sont proposées même hors du PATH.
        let completer = Completer::builtin().with_custom(vec![custom(
            r#"{"names": ["easytab-perso"], "options": [{"names": ["--perso"]}]}"#,
        )]);
        assert_eq!(labels_of(&completer, "easytab-pers"), ["easytab-perso"]);
        assert_eq!(labels_of(&completer, "easytab-perso --"), ["--perso"]);
    }

    /// `--help` ne sert qu'aux commandes sans spec.
    #[cfg(unix)]
    #[test]
    fn help_is_a_fallback_for_commands_without_spec() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::mpsc;

        let dir = temp_dir("aide");
        for name in ["git", "easytab-sans-spec"] {
            let path = dir.join(name);
            std::fs::write(
                &path,
                "#!/bin/sh\necho '  -a, --aide-lue  lue'\necho '  -b  autre'\n",
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let (ready, notified) = mpsc::channel();
        let completer =
            Completer::builtin().with_help(HelpSpecs::in_dirs(vec![dir.clone()], move || {
                let _ = ready.send(());
            }));
        // `git` a une spec : son `--help` n'est pas lu.
        let found = labels_of(&completer, "git --aide");
        assert!(!found.contains(&"-a, --aide-lue".to_string()), "{found:?}");
        assert!(notified.try_recv().is_err());

        assert!(labels_of(&completer, "easytab-sans-spec --").is_empty());
        notified
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            labels_of(&completer, "easytab-sans-spec --aide"),
            ["-a, --aide-lue"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
