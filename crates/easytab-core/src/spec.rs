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
    /// [`Modules`]). Seulement sur une spec de premier niveau ou chargeable.
    pub module: Option<String>,
    /// Le contenu de la commande est celui d'une autre spec (`loadSpec` de
    /// Fig) : une spec chargeable (`php/bin-console`, voir [`loadable`]) ou de
    /// premier niveau (`git`).
    pub load: Option<String>,
    /// Chemin, dans le module, d'une fonction `generateSpec` qui calcule des
    /// sous-commandes et options à ajouter à celles-ci (`[]` : la spec
    /// elle-même).
    pub generate: Option<Vec<PathKey>>,
}

impl Command {
    /// Ajoute ce que `generateSpec` a produit, comme Fig : les sous-commandes et
    /// options de même nom sont remplacées, les autres ajoutées ; les arguments
    /// générés, s'il y en a, remplacent les autres.
    pub fn merge(&mut self, generated: &Command) {
        for sub in &generated.subcommands {
            match self
                .subcommands
                .iter_mut()
                .find(|s| s.names.iter().any(|n| sub.names.contains(n)))
            {
                Some(existing) => *existing = sub.clone(),
                None => self.subcommands.push(sub.clone()),
            }
        }
        for opt in &generated.options {
            match self
                .options
                .iter_mut()
                .find(|o| o.names.iter().any(|n| opt.names.contains(n)))
            {
                Some(existing) => *existing = opt.clone(),
                None => self.options.push(opt.clone()),
            }
        }
        if !generated.args.is_empty() {
            self.args = generated.args.clone();
        }
        if self.description.is_none() {
            self.description = generated.description.clone();
        }
    }
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
    /// Après cet argument, la ligne se complète avec cette spec (`loadSpec`
    /// d'un argument), désignée par son chemin comme [`Command::load`]…
    pub load: Option<String>,
    /// … ou donnée sur place.
    pub spec: Option<Box<Command>>,
}

/// Generator Fig : suggestions calculées par le JavaScript de la spec.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Generator {
    /// Chemin du generator dans l'objet exporté par le module de la spec.
    pub path: Vec<PathKey>,
    /// Module du generator, quand ce n'est pas celui de la spec : pour une
    /// spec produite par `generateSpec`, l'objet qu'elle a produit.
    pub module: Option<String>,
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
struct Bundle<T> {
    specs: T,
}

/// Specs embarquées dans le binaire (`specs/specs.json.z`, JSON compressé).
pub fn builtin() -> Vec<Spec> {
    let json = inflate(include_bytes!("../../../specs/specs.json.z"));
    let bundle: Bundle<Vec<Box<RawValue>>> =
        serde_json::from_slice(&json).expect("specs/specs.json.z invalide");
    bundle.specs.into_iter().map(Spec::from).collect()
}

/// Specs embarquées que d'autres chargent par `loadSpec`, par chemin
/// (`specs/loadable.json.z`). Décompressées seulement au premier `loadSpec`.
pub fn loadable() -> HashMap<String, Spec> {
    let json = inflate(include_bytes!("../../../specs/loadable.json.z"));
    let bundle: Bundle<HashMap<String, Box<RawValue>>> =
        serde_json::from_slice(&json).expect("specs/loadable.json.z invalide");
    bundle
        .specs
        .into_iter()
        .map(|(path, raw)| (path, Spec::from(raw)))
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn command(json: &str) -> Command {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn merges_generated_content() {
        let mut spec = command(
            r#"{"names": ["outil"], "description": "statique",
                "subcommands": [{"names": ["a"], "description": "avant"}, {"names": ["b"]}],
                "options": [{"names": ["-v", "--verbose"]}],
                "args": [{"name": "statique"}]}"#,
        );
        spec.merge(&command(
            r#"{"names": ["outil"], "description": "générée",
                "subcommands": [{"names": ["a"], "description": "après"}, {"names": ["c"]}],
                "options": [{"names": ["--verbose"], "description": "nouvelle"}, {"names": ["-q"]}],
                "args": [{"name": "générée"}]}"#,
        ));
        let subs: Vec<_> = spec
            .subcommands
            .iter()
            .map(|s| (s.names[0].as_str(), s.description.as_deref()))
            .collect();
        assert_eq!(subs, [("a", Some("après")), ("b", None), ("c", None)]);
        let options: Vec<_> = spec.options.iter().map(|o| o.names.join(",")).collect();
        assert_eq!(options, ["--verbose", "-q"]);
        assert_eq!(spec.args[0].name.as_deref(), Some("générée"));
        assert_eq!(spec.description.as_deref(), Some("statique"));
    }

    #[test]
    fn embeds_loadable_specs() {
        let loadable = loadable();
        let console = loadable["php/bin-console"].command();
        assert_eq!(console.module.as_deref(), Some("php/bin-console"));
        assert_eq!(console.generate.as_deref(), Some(&[][..]));
        assert!(loadable.contains_key("dotnet/dotnet-build"));
        // Pas les specs de services cloud (voir `tools/import-fig-specs.mjs`).
        assert!(loadable.keys().all(|path| !path.starts_with("aws/")));
        // Le code de `generateSpec` est embarqué.
        assert!(Modules::builtin().modules.contains_key("php/bin-console"));
    }
}
