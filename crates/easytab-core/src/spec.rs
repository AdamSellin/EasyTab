//! Specs de complétion : description déclarative des sous-commandes, options et
//! arguments de chaque CLI. Le format est celui produit par
//! `tools/import-fig-specs.mjs` à partir des specs Fig.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Command {
    pub names: Vec<String>,
    pub description: Option<String>,
    pub subcommands: Vec<Command>,
    pub options: Vec<Opt>,
    pub args: Vec<Arg>,
    pub hidden: bool,
    pub insert: Option<String>,
    /// Module JavaScript d'où viennent les generators de la spec (voir
    /// [`Modules`]). Seulement sur une spec de premier niveau.
    pub module: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Opt {
    pub names: Vec<String>,
    pub description: Option<String>,
    pub args: Vec<Arg>,
    /// L'option reste valable dans les sous-commandes.
    pub persistent: bool,
    pub repeatable: bool,
    /// La valeur s'écrit collée : `--color=auto`.
    pub requires_equals: bool,
    pub hidden: bool,
    pub insert: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Arg {
    pub name: Option<String>,
    pub description: Option<String>,
    pub optional: bool,
    pub variadic: bool,
    /// L'argument est lui-même une commande (`sudo`, `npx`…).
    pub is_command: bool,
    pub suggestions: Vec<Value>,
    pub templates: Vec<Template>,
    pub generators: Vec<Generator>,
}

/// Generator Fig : suggestions calculées par le JavaScript de la spec.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Generator {
    /// Chemin du generator dans l'objet exporté par le module de la spec.
    pub path: Vec<PathKey>,
    /// Les résultats sont recalculés quand ce texte apparaît dans le mot en cours.
    pub trigger: Option<String>,
    /// Seule la partie du mot après la dernière occurrence de ce texte sert à
    /// filtrer les résultats (et est remplacée à l'insertion).
    pub query_term: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PathKey {
    Index(usize),
    Key(String),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Value {
    pub names: Vec<String>,
    pub description: Option<String>,
    pub insert: Option<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Template {
    Filepaths,
    Folders,
}

/// Spec de premier niveau, lue à la demande : seuls ses noms et sa description
/// sont lus au démarrage, le reste à la première complétion de la commande.
#[derive(Debug)]
pub struct Spec {
    pub names: Vec<String>,
    pub description: Option<String>,
    raw: Option<Box<RawValue>>,
    command: OnceLock<Command>,
}

impl Spec {
    pub fn command(&self) -> &Command {
        self.command.get_or_init(|| {
            self.raw
                .as_ref()
                .and_then(|raw| serde_json::from_str(raw.get()).ok())
                .unwrap_or_default()
        })
    }
}

impl From<Command> for Spec {
    fn from(command: Command) -> Self {
        Self {
            names: command.names.clone(),
            description: command.description.clone(),
            raw: None,
            command: OnceLock::from(command),
        }
    }
}

impl From<Box<RawValue>> for Spec {
    fn from(raw: Box<RawValue>) -> Self {
        #[derive(Deserialize)]
        struct Head {
            #[serde(default)]
            names: Vec<String>,
            description: Option<String>,
        }
        let head: Head = serde_json::from_str(raw.get()).unwrap_or(Head {
            names: Vec::new(),
            description: None,
        });
        Self {
            names: head.names,
            description: head.description,
            raw: Some(raw),
            command: OnceLock::new(),
        }
    }
}

#[derive(Deserialize)]
struct Bundle {
    specs: Vec<Box<RawValue>>,
}

/// Specs embarquées dans le binaire (`specs/specs.json.z`, JSON compressé).
pub fn builtin() -> Vec<Spec> {
    let json = inflate(include_bytes!("../../../specs/specs.json.z"));
    let bundle: Bundle = serde_json::from_slice(&json).expect("specs/specs.json.z invalide");
    bundle.specs.into_iter().map(Spec::from).collect()
}

fn inflate(data: &[u8]) -> Vec<u8> {
    miniz_oxide::inflate::decompress_to_vec_zlib(data).expect("specs embarquées illisibles")
}

/// Code JavaScript des specs qui ont des generators, par nom de module.
#[derive(Debug, Deserialize)]
pub struct Modules {
    pub source: String,
    pub modules: HashMap<String, String>,
}

impl Modules {
    /// Modules embarqués dans le binaire (`specs/modules.json.z`).
    pub fn builtin() -> Self {
        let json = inflate(include_bytes!("../../../specs/modules.json.z"));
        serde_json::from_slice(&json).expect("specs/modules.json.z invalide")
    }
}
