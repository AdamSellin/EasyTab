//! Commandes PowerShell : PowerShell décrit lui-même ses commandes
//! (`Get-Command`). Sans spec à écrire, EasyTab lui demande la liste des
//! commandes, puis les paramètres de chacune à sa première utilisation
//! (`Remove-Item -Recurse`, `-Path`…). Les réponses sont gardées dans
//! `~/.easytab/cache/powershell.json` : PowerShell met plusieurs centaines de
//! millisecondes à démarrer, la frappe ne l'attend jamais.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::exec;
use crate::spec::{Arg, Command, Opt, Template, Value};

/// Une interrogation de PowerShell, démarrage compris.
const TIMEOUT: Duration = Duration::from_secs(20);
/// La liste des commandes est redemandée au-delà (modules installés depuis).
const COMMANDS_FRESH_FOR: Duration = Duration::from_secs(24 * 3600);

/// Liste des commandes : nom et type (`Cmdlet`, `Function`, `Alias`…).
const LIST_SCRIPT: &str = "Get-Command -CommandType Cmdlet,Function,Alias \
     | ForEach-Object { @{ n = $_.Name; t = [string]$_.CommandType } } \
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
}

/// Ce qui est gardé sur disque.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    commands: Vec<String>,
    /// Date de la liste, en secondes depuis 1970.
    listed_at: u64,
    /// Paramètres par nom de commande en minuscules.
    params: HashMap<String, Vec<Param>>,
}

#[derive(Default)]
struct State {
    cache: Cache,
    /// Commandes connues, en minuscules.
    known: HashSet<String>,
    /// Specs déjà construites (une par commande utilisée, gardées jusqu'à la
    /// fin du programme).
    specs: HashMap<String, &'static Command>,
    running: HashSet<String>,
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
        let stale = SystemTime::now()
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

    fn set_cache(&self, cache: Cache) {
        let mut state = self.state.lock().unwrap();
        state.known = cache.commands.iter().map(|c| c.to_lowercase()).collect();
        state.cache = cache;
    }

    /// Noms des commandes PowerShell connues.
    pub fn commands(&self) -> Vec<String> {
        self.state.lock().unwrap().cache.commands.clone()
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
        if !state.known.contains(&key) || !valid_name(name) {
            return None;
        }
        let Some(params) = state.cache.params.get(&key) else {
            drop(state);
            self.fetch(name, Job::Params);
            return None;
        };
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
            let script = match job {
                Job::List => LIST_SCRIPT.to_string(),
                Job::Params => PARAMS_SCRIPT.replace("{name}", &name),
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
                TIMEOUT,
            );
            {
                let mut state = shell.state.lock().unwrap();
                state.running.remove(&id);
                match job {
                    Job::List => {
                        let Some(commands) = parse_list(&output.stdout) else {
                            return;
                        };
                        state.known = commands.iter().map(|c| c.to_lowercase()).collect();
                        state.cache.commands = commands;
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

fn parse_list(json: &str) -> Option<Vec<String>> {
    let listed: Vec<Listed> = one_or_many(json)?;
    let mut names: Vec<String> = listed
        .into_iter()
        // Les alias d'une lettre (`%`, `?`) et ceux qui imitent Unix (`ls`,
        // `rm`) sont déjà couverts par les specs ou n'aident pas.
        .filter(|c| c.kind != "Alias" || c.name.contains('-'))
        .map(|c| c.name)
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    Some(names)
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
        let list = parse_list(
            r#"[{"n":"Remove-Item","t":"Cmdlet"},{"n":"ls","t":"Alias"},
                {"n":"Get-ChildItem","t":"Cmdlet"},{"n":"Set-Location","t":"Cmdlet"}]"#,
        )
        .unwrap();
        assert_eq!(list, ["Get-ChildItem", "Remove-Item", "Set-Location"]);
        // Une seule commande : ConvertTo-Json ne fait pas de tableau.
        assert_eq!(
            parse_list(r#"{"n":"Get-Date","t":"Cmdlet"}"#).unwrap(),
            ["Get-Date"]
        );
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
    }
}
