//! Specs de complétion : description déclarative des sous-commandes, options et
//! arguments de chaque CLI. Le format est celui produit par
//! `tools/import-fig-specs.mjs` à partir des specs Fig.

use serde::Deserialize;

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
