//! Specs de complétion : description déclarative des sous-commandes, options et
//! arguments de chaque CLI. Le format est celui produit par
//! `tools/import-fig-specs.mjs` à partir des specs Fig.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Deserialize)]
pub struct Bundle {
    pub source: String,
    pub specs: Vec<Command>,
}

impl Bundle {
    /// Specs embarquées dans le binaire (`specs/specs.json`).
    pub fn builtin() -> Self {
        serde_json::from_str(include_str!("../../../specs/specs.json"))
            .expect("specs/specs.json invalide")
    }
}

/// Code JavaScript des specs qui ont des generators, par nom de module.
#[derive(Debug, Deserialize)]
pub struct Modules {
    pub source: String,
    pub modules: HashMap<String, String>,
}

impl Modules {
    /// Modules embarqués dans le binaire (`specs/modules.json`).
    pub fn builtin() -> Self {
        serde_json::from_str(include_str!("../../../specs/modules.json"))
            .expect("specs/modules.json invalide")
    }
}
