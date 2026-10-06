//! Commandes PowerShell : PowerShell décrit lui-même ses commandes
//! (`Get-Command`). Sans spec à écrire, EasyTab lui demande la liste des
//! commandes, puis les paramètres de chacune à sa première utilisation
//! (`Remove-Item -Recurse`, `-Path`…). Les réponses sont gardées dans
//! `~/.easytab/cache/powershell.json` : PowerShell met plusieurs centaines de
//! millisecondes à démarrer, la frappe ne l'attend jamais.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::exec;
use crate::spec::{Arg, Command, Opt, Template, Value};

/// Paramètres d'une commande, démarrage de PowerShell compris.
const TIMEOUT: Duration = Duration::from_secs(20);
/// La liste complète : PowerShell parcourt alors tous les modules installés,
/// ce qui prend parfois plus d'une demi-minute.
const LIST_TIMEOUT: Duration = Duration::from_secs(90);
/// La liste des commandes est redemandée au-delà (modules installés depuis).
const COMMANDS_FRESH_FOR: Duration = Duration::from_secs(24 * 3600);

/// Liste des commandes : nom, type (`Cmdlet`, `Function`, `Alias`…) et, pour
/// un alias, la commande qu'il désigne.
const LIST_SCRIPT: &str = "Get-Command -CommandType Cmdlet,Function,Alias \
     | ForEach-Object { @{ n = $_.Name; t = [string]$_.CommandType; \
     r = $(if ($_.CommandType -eq 'Alias') { $_.Definition }) } } \
     | ConvertTo-Json -Compress";

/// Paramètres d'une commande (`{name}` est remplacé, après vérification).
const PARAMS_SCRIPT: &str = "$c = Get-Command '{name}' -ErrorAction Stop; \
     if ($c.CommandType -eq 'Alias') { $c = $c.ResolvedCommand }; \
     $common = [System.Management.Automation.Cmdlet]::CommonParameters + \
     [System.Management.Automation.Cmdlet]::OptionalCommonParameters; \
     @($c.Parameters.Values | Where-Object { $common -notcontains $_.Name } | ForEach-Object { \
     @{ n = $_.Name; a = @($_.Aliases); s = [bool]$_.SwitchParameter; t = $_.ParameterType.Name; \
     v = @($_.Attributes | Where-Object { $_ -is [System.Management.Automation.ValidateSetAttribute] } \
     | ForEach-Object { $_.ValidValues }) } }) | ConvertTo-Json -Compress -Depth 3";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    #[serde(rename = "n")]
    pub name: String,
    #[serde(rename = "a", default)]
    pub aliases: Vec<String>,
    /// Paramètre sans valeur (`-Recurse`).
    #[serde(rename = "s", default)]
    pub switch: bool,
    /// Type .NET de la valeur (`String`, `String[]`…).
    #[serde(rename = "t", default)]
    pub type_name: String,
    /// Valeurs permises (`ValidateSet`).
    #[serde(rename = "v", default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Listed {
    #[serde(rename = "n")]
    name: String,
    #[serde(rename = "t", default)]
    kind: String,
    /// Commande désignée par un alias.
    #[serde(rename = "r", default)]
    target: Option<String>,
}

/// Ce qui est gardé sur disque.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    commands: Vec<String>,
    /// Alias (`ls`, `gci`) et la commande qu'ils désignent (`Get-ChildItem`).
    /// Absent d'un cache écrit avant leur prise en charge : la liste est
    /// alors redemandée.
    #[serde(default)]
    aliases: Option<BTreeMap<String, String>>,
    /// Date de la liste, en secondes depuis 1970.
    listed_at: u64,
    /// Paramètres par nom de commande en minuscules.
    params: HashMap<String, Vec<Param>>,
}

#[derive(Default)]
struct State {
    cache: Cache,
    /// Commandes connues (alias compris), en minuscules.
    known: HashSet<String>,
    /// Commande désignée par chaque alias, par alias en minuscules.
    targets: HashMap<String, String>,
    /// Specs déjà construites (une par commande utilisée, gardées jusqu'à la
    /// fin du programme).
    specs: HashMap<String, &'static Command>,
    running: HashSet<String>,
}

impl State {
    /// Recalcule les commandes connues et les cibles des alias.
    fn index(&mut self) {
        let aliases = self.cache.aliases.iter().flatten();
        self.targets = aliases
            .map(|(alias, target)| (alias.to_lowercase(), target.clone()))
            .collect();
        self.known = self
            .cache
            .commands
            .iter()
            .map(|c| c.to_lowercase())
            .chain(self.targets.keys().cloned())
            .collect();
    }
}

#[derive(Clone)]
pub struct PowerShell {
    program: String,
    cache_file: Option<PathBuf>,
    state: Arc<Mutex<State>>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl PowerShell {
    /// `program` : `pwsh` ou `powershell`, celui que l'utilisateur lance.
    /// `notify` est appelé quand de nouvelles réponses arrivent.
    pub fn start(program: &str, notify: impl Fn() + Send + Sync + 'static) -> Self {
        let cache_file = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(|home| {
                PathBuf::from(home)
                    .join(".easytab")
                    .join("cache")
                    .join("powershell.json")
            });
        let cache: Cache = cache_file
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        let shell = Self {
            program: program.to_string(),
            cache_file,
            state: Arc::new(Mutex::new(State::default())),
            notify: Arc::new(notify),
        };
        let stale = cache.aliases.is_none()
            || SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(true, |now| {
                    now.as_secs().saturating_sub(cache.listed_at) > COMMANDS_FRESH_FOR.as_secs()
                });
        shell.set_cache(cache);
        if stale {
            shell.fetch("", Job::List);
        }
        shell
    }

    /// PowerShell déjà interrogé : `list` est la réponse de [`LIST_SCRIPT`],
    /// `params` celles de [`PARAMS_SCRIPT`] par commande.
    #[cfg(test)]
    pub(crate) fn answered(list: &str, params: &[(&str, &str)]) -> Self {
        let (commands, aliases) = parse_list(list).unwrap();
        let params = params
            .iter()
            .map(|(name, json)| (name.to_lowercase(), parse_params(json).unwrap()))
            .collect();
        let shell = Self {
            program: String::new(),
            cache_file: None,
            state: Arc::default(),
            notify: Arc::new(|| {}),
        };
        shell.set_cache(Cache {
            commands,
            aliases: Some(aliases),
            listed_at: u64::MAX,
            params,
        });
        shell
    }

    fn set_cache(&self, cache: Cache) {
        let mut state = self.state.lock().unwrap();
        state.cache = cache;
        state.index();
    }

    /// Commandes PowerShell connues, avec, pour un alias, la commande qu'il
    /// désigne (`ls` → `Get-ChildItem`).
    pub fn commands(&self) -> Vec<(String, Option<String>)> {
        let state = self.state.lock().unwrap();
        let commands = state.cache.commands.iter().map(|c| (c.clone(), None));
        let aliases = state
            .cache
            .aliases
            .iter()
            .flatten()
            .map(|(alias, target)| (alias.clone(), Some(target.clone())));
        commands.chain(aliases).collect()
    }

    /// Commande désignée par l'alias `name` (`gci` → `Get-ChildItem`).
    pub fn alias_target(&self, name: &str) -> Option<String> {
        let state = self.state.lock().unwrap();
        state.targets.get(&name.to_lowercase()).cloned()
    }

    /// Alias connus, en minuscules.
    pub fn aliases(&self) -> HashSet<String> {
        self.state.lock().unwrap().targets.keys().cloned().collect()
    }

    /// Spec de la commande PowerShell `name`, construite à partir de ses
    /// paramètres. `None` si ce n'en est pas une, ou tant que PowerShell
    /// n'a pas répondu (la réponse déclenche `notify`).
    pub fn spec(&self, name: &str) -> Option<&'static Command> {
        let key = name.to_lowercase();
        let mut state = self.state.lock().unwrap();
        if let Some(spec) = state.specs.get(&key) {
            return Some(spec);
        }
        if !valid_name(name) {
            return None;
        }
        // Tant que la liste n'est pas arrivée, les noms en `Verbe-Nom` sont
        // demandés directement : la liste peut être longue à venir.
        let known = state.known.contains(&key);
        let listed = !state.cache.commands.is_empty();
        if !known && (listed || !is_verb_noun(name)) {
            return None;
        }
        // Un alias reçoit les paramètres de sa commande (`ls -Recurse`),
        // demandés une seule fois pour les deux.
        let target = state.targets.get(&key).cloned();
        let target = target.as_deref().unwrap_or(name);
        if !valid_name(target) {
            return None;
        }
        let Some(params) = state.cache.params.get(&target.to_lowercase()) else {
            drop(state);
            self.fetch(target, Job::Params);
            return None;
        };
        // Pas une commande PowerShell (ou sans paramètre) : rien à proposer.
        if params.is_empty() && !known {
            return None;
        }
        let spec: &'static Command = Box::leak(Box::new(command_spec(name, params)));
        state.specs.insert(key, spec);
        Some(spec)
    }

    /// Interroge PowerShell en arrière-plan.
    fn fetch(&self, name: &str, job: Job) {
        let id = match job {
            Job::List => String::new(),
            Job::Params => name.to_lowercase(),
        };
        if !self.state.lock().unwrap().running.insert(id.clone()) {
            return;
        }
        let shell = self.clone();
        let name = name.to_string();
        thread::spawn(move || {
            let (script, timeout) = match job {
                Job::List => (LIST_SCRIPT.to_string(), LIST_TIMEOUT),
                Job::Params => (PARAMS_SCRIPT.replace("{name}", &name), TIMEOUT),
            };
            let output = exec::run(
                &shell.program,
                &[
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    script,
                ],
                &std::env::temp_dir(),
                timeout,
            );
            if output.status != 0 {
                log_failure(&name, &output);
            }
            {
                let mut state = shell.state.lock().unwrap();
                state.running.remove(&id);
                match job {
                    Job::List => {
                        let Some((commands, aliases)) = parse_list(&output.stdout) else {
                            return;
                        };
                        state.cache.commands = commands;
                        state.cache.aliases = Some(aliases);
                        state.index();
                        state.cache.listed_at = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs());
                    }
                    Job::Params => {
                        // Une commande sans paramètre, ou en erreur : liste vide,
                        // pour ne pas redemander à chaque frappe.
                        let params = parse_params(&output.stdout).unwrap_or_default();
                        state.cache.params.insert(id, params);
                    }
                }
                if let Some(path) = &shell.cache_file {
                    save(path, &state.cache);
                }
            }
            (shell.notify)();
        });
    }
}

#[derive(Clone, Copy)]
enum Job {
    List,
    Params,
}

/// `Get-ChildItem`, `Remove-Item` : forme des cmdlets.
fn is_verb_noun(name: &str) -> bool {
    matches!(name.split_once('-'), Some((verb, noun)) if !verb.is_empty() && !noun.is_empty())
}

/// Échec de PowerShell, noté dans le journal de diagnostic (`EASYTAB_LOG`).
fn log_failure(name: &str, output: &exec::Output) {
    use std::io::Write;
    let Some(path) = std::env::var_os("EASYTAB_LOG") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(
            file,
            "PowerShell {name:?} : code {}, {}",
            output.status,
            output.stderr.trim()
        );
    }
}

/// Seuls les noms de commande simples sont passés à PowerShell.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn save(path: &Path, cache: &Cache) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_vec(cache) {
        let _ = std::fs::write(path, data);
    }
}

/// `ConvertTo-Json` donne un objet seul au lieu d'un tableau d'un élément.
fn one_or_many<T: for<'de> Deserialize<'de>>(json: &str) -> Option<Vec<T>> {
    let json = json.trim();
    if json.is_empty() {
        return Some(Vec::new());
    }
    serde_json::from_str::<Vec<T>>(json)
        .ok()
        .or_else(|| serde_json::from_str::<T>(json).ok().map(|one| vec![one]))
}

/// Commandes, puis alias avec la commande qu'ils désignent. Les alias faits
/// de symboles (`%`, `?`) sont écartés.
fn parse_list(json: &str) -> Option<(Vec<String>, BTreeMap<String, String>)> {
    let listed: Vec<Listed> = one_or_many(json)?;
    let mut names = Vec::new();
    let mut aliases = BTreeMap::new();
    for command in listed {
        if command.kind != "Alias" {
            names.push(command.name);
        } else if let Some(target) = command.target.filter(|t| !t.is_empty()) {
            if valid_name(&command.name) {
                aliases.insert(command.name, target);
            }
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    Some((names, aliases))
}

fn parse_params(json: &str) -> Option<Vec<Param>> {
    one_or_many(json)
}

/// Spec construite à partir des paramètres : chaque paramètre devient une
/// option `-Nom` (et ses alias), avec ses valeurs permises ou des chemins.
pub fn command_spec(name: &str, params: &[Param]) -> Command {
    let options = params
        .iter()
        .map(|param| Opt {
            names: std::iter::once(&param.name)
                .chain(&param.aliases)
                .map(|n| format!("-{n}"))
                .collect(),
            description: (!param.switch && !param.type_name.is_empty())
                .then(|| format!("<{}>", param.type_name)),
            args: if param.switch {
                Vec::new()
            } else {
                vec![value_arg(param)]
            },
            ..Opt::default()
        })
        .collect();
    let takes_path = params.iter().any(|p| is_path(&p.name));
    Command {
        names: vec![name.to_string()],
        options,
        // Premier argument sans nom : un chemin pour les commandes de fichiers.
        args: if takes_path {
            vec![Arg {
                optional: true,
                templates: vec![Template::Filepaths],
                ..Arg::default()
            }]
        } else {
            Vec::new()
        },
        ..Command::default()
    }
}

fn is_path(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "path" | "literalpath" | "filepath" | "destination"
    )
}

fn value_arg(param: &Param) -> Arg {
    Arg {
        name: Some(param.name.clone()),
        suggestions: param
            .values
            .iter()
            .map(|value| Value {
                names: vec![value.clone()],
                ..Value::default()
            })
            .collect(),
        templates: if is_path(&param.name) {
            vec![Template::Filepaths]
        } else {
            Vec::new()
        },
        ..Arg::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_command_list() {
        let (list, aliases) = parse_list(
            r#"[{"n":"Remove-Item","t":"Cmdlet","r":null},{"n":"ls","t":"Alias","r":"Get-ChildItem"},
                {"n":"Get-ChildItem","t":"Cmdlet"},{"n":"Set-Location","t":"Cmdlet"},
                {"n":"cd","t":"Alias","r":"Set-Location"},{"n":"?","t":"Alias","r":"Where-Object"},
                {"n":"vide","t":"Alias"}]"#,
        )
        .unwrap();
        assert_eq!(list, ["Get-ChildItem", "Remove-Item", "Set-Location"]);
        let aliases: Vec<_> = aliases
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        assert_eq!(aliases, [("cd", "Set-Location"), ("ls", "Get-ChildItem")]);
        // Une seule commande : ConvertTo-Json ne fait pas de tableau.
        assert_eq!(
            parse_list(r#"{"n":"Get-Date","t":"Cmdlet"}"#).unwrap().0,
            ["Get-Date"]
        );
    }

    #[test]
    fn aliases_share_their_command_parameters() {
        let shell = PowerShell::answered(
            r#"[{"n":"Get-Content","t":"Cmdlet"},{"n":"cat","t":"Alias","r":"Get-Content"},
                {"n":"gc","t":"Alias","r":"Get-Content"}]"#,
            &[(
                "Get-Content",
                r#"[{"n":"Path","a":[],"s":false,"t":"String[]","v":[]}]"#,
            )],
        );
        assert_eq!(shell.alias_target("GC").as_deref(), Some("Get-Content"));
        assert_eq!(shell.alias_target("Get-Content"), None);
        let spec = shell.spec("cat").unwrap();
        assert_eq!(spec.names, ["cat"]);
        assert_eq!(spec.options[0].names, ["-Path"]);
        assert!(shell.spec("gc").is_some());
        let commands = shell.commands();
        assert!(commands.contains(&("gc".to_string(), Some("Get-Content".to_string()))));
        assert!(commands.contains(&("Get-Content".to_string(), None)));
        // Un cache d'avant les alias est redemandé.
        let old: Cache =
            serde_json::from_str(r#"{"commands":[],"listed_at":0,"params":{}}"#).unwrap();
        assert!(old.aliases.is_none());
    }

    #[test]
    fn builds_a_spec_from_parameters() {
        let params = parse_params(
            r#"[{"n":"Path","a":[],"s":false,"t":"String[]","v":[]},
                {"n":"Recurse","a":[],"s":true,"t":"SwitchParameter","v":[]},
                {"n":"Force","a":[],"s":true,"t":"SwitchParameter","v":[]},
                {"n":"Encoding","a":["enc"],"s":false,"t":"String","v":["utf8","ascii"]}]"#,
        )
        .unwrap();
        let spec = command_spec("Remove-Item", &params);
        let names: Vec<&str> = spec
            .options
            .iter()
            .flat_map(|o| o.names.iter().map(String::as_str))
            .collect();
        assert_eq!(names, ["-Path", "-Recurse", "-Force", "-Encoding", "-enc"]);
        assert!(spec.options[1].args.is_empty());
        assert_eq!(spec.options[0].args[0].templates, [Template::Filepaths]);
        assert_eq!(spec.options[3].args[0].suggestions[1].names, ["ascii"]);
        assert_eq!(spec.args[0].templates, [Template::Filepaths]);
    }

    #[test]
    fn refuses_odd_names() {
        assert!(valid_name("Remove-Item"));
        assert!(valid_name("Microsoft.PowerShell.Core"));
        assert!(!valid_name("x'; rm -r /; '"));
        assert!(!valid_name(""));
        assert!(is_verb_noun("Remove-Item"));
        assert!(!is_verb_noun("git"));
        assert!(!is_verb_noun("-Recurse"));
    }
}
