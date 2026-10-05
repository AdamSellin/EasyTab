//! Generators Fig : suggestions calculées en lançant une commande (branches git,
//! scripts npm, conteneurs docker…) puis en passant sa sortie au JavaScript de
//! la spec. Aussi les specs dont une partie est calculée par `generateSpec`
//! (commandes de `composer`, scripts de `bin/console`…).
//!
//! Ils tournent dans un fil à part, avec le moteur JS embarqué (QuickJS), pour
//! ne jamais bloquer la frappe. Les résultats sont gardés en cache : la liste
//! affiche ce qui est déjà connu, et le fil prévient quand du nouveau arrive.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rquickjs::{CatchResultExt, Context, Function, Module, Object, Promise, Runtime, Value};
use serde::Deserialize;

use crate::exec;
use crate::spec::{Command, Generator, Modules, PathKey};

/// Au-delà, les résultats sont recalculés (en gardant les anciens à l'écran).
const FRESH_FOR: Duration = Duration::from_secs(5);
/// Pareil pour les specs produites par `generateSpec` (liste des commandes de
/// `composer`…), plus coûteuses et qui changent rarement.
const SPEC_FRESH_FOR: Duration = Duration::from_secs(60);
/// Durée maximale d'une commande lancée par un generator.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// Durée maximale d'un generator, JavaScript compris.
const RUN_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CACHE_ENTRIES: usize = 256;
/// Le moteur JS garde autant de specs générées (voir `generators.js`).
const MAX_SPEC_ENTRIES: usize = 32;
/// Marque les modules qui sont des specs générées, et non des sources.
const GENERATED: &str = "#generated:";

/// Suggestion produite par un generator.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Item {
    pub names: Vec<String>,
    pub insert: Option<String>,
    pub description: Option<String>,
    /// Icône Fig (`fig://icon?type=git` donne `git`).
    #[serde(default)]
    pub icon: Option<String>,
    pub priority: i64,
}

/// Ce qui identifie un calcul : le même generator, au même endroit, avec les
/// mêmes mots avant le mot en cours, donne les mêmes résultats.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    module: String,
    path: Vec<PathKey>,
    cwd: PathBuf,
    context: Vec<String>,
}

#[derive(Default)]
struct Entry {
    items: Option<Arc<Vec<Item>>>,
    fetched: Option<Instant>,
    running: bool,
}

/// Une spec générée : par `generateSpec` au chemin `path` du module, dans un
/// dossier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SpecKey {
    module: String,
    path: Vec<PathKey>,
    cwd: PathBuf,
}

impl SpecKey {
    /// Nom sous lequel le moteur JS garde l'objet produit, et que portent les
    /// generators de la spec convertie.
    fn generated_module(&self) -> String {
        let path = serde_json::to_string(&self.path).unwrap_or_default();
        format!("{}{GENERATED}{path}:{}", self.module, self.cwd.display())
    }
}

#[derive(Default)]
struct SpecEntry {
    /// Ce que `generateSpec` a produit (`None` à l'intérieur : rien d'utile).
    generated: Option<Option<Arc<Command>>>,
    /// La spec statique complétée par `generated`, calculée à la demande.
    merged: Option<Arc<Command>>,
    fetched: Option<Instant>,
    running: bool,
}

enum Job {
    Generator { key: Key, tokens: Vec<String> },
    Spec { key: SpecKey, tokens: Vec<String> },
}

/// Résultat de [`Generators::spec`].
#[derive(Debug, Clone)]
pub(crate) enum Generated {
    /// La spec complétée par `generateSpec`, ou `None` s'il n'a rien donné.
    Ready(Option<Arc<Command>>),
    /// En cours de calcul : compléter avec la spec statique en attendant.
    Pending,
}

type Cache<K, V> = Arc<Mutex<HashMap<K, V>>>;

#[derive(Clone, Default)]
pub struct Generators {
    worker: Option<Sender<Job>>,
    cache: Cache<Key, Entry>,
    specs: Cache<SpecKey, SpecEntry>,
}

impl Generators {
    /// Sans fil de calcul : les generators ne donnent rien.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Démarre le fil de calcul avec les modules embarqués. `notify` est appelé
    /// (depuis ce fil) chaque fois que de nouveaux résultats sont prêts.
    pub fn start(notify: impl Fn() + Send + 'static) -> Self {
        Self::spawn(|| Modules::builtin().modules, notify)
    }

    #[cfg(all(test, unix))]
    pub(crate) fn start_with(
        modules: HashMap<String, String>,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        Self::spawn(move || modules, notify)
    }

    /// `load` est appelé dans le fil de calcul : lire les modules ne retarde pas
    /// le démarrage du terminal.
    fn spawn(
        load: impl FnOnce() -> HashMap<String, String> + Send + 'static,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let cache: Cache<Key, Entry> = Arc::default();
        let specs: Cache<SpecKey, SpecEntry> = Arc::default();
        let (worker, jobs) = mpsc::channel::<Job>();
        let (worker_cache, worker_specs) = (Arc::clone(&cache), Arc::clone(&specs));
        thread::Builder::new()
            .name("easytab-generators".into())
            .spawn(move || {
                let mut engine = Engine::new(load());
                for job in jobs {
                    match job {
                        Job::Generator { key, tokens } => {
                            let items = match &mut engine {
                                Ok(engine) => engine
                                    .run(&key.module, &key.path, &tokens, &key.cwd)
                                    .unwrap_or_default(),
                                Err(_) => Vec::new(),
                            };
                            let mut cache = worker_cache.lock().unwrap();
                            let entry = cache.entry(key).or_default();
                            entry.items = Some(Arc::new(items));
                            entry.fetched = Some(Instant::now());
                            entry.running = false;
                        }
                        Job::Spec { key, tokens } => {
                            let generated = match &mut engine {
                                Ok(engine) => engine.generate(&key, &tokens).ok().flatten(),
                                Err(_) => None,
                            };
                            let mut specs = worker_specs.lock().unwrap();
                            let entry = specs.entry(key).or_default();
                            entry.generated = Some(generated.map(Arc::new));
                            entry.merged = None;
                            entry.fetched = Some(Instant::now());
                            entry.running = false;
                        }
                    }
                    notify();
                }
            })
            .expect("lancement du fil des generators");
        Self {
            worker: Some(worker),
            cache,
            specs,
        }
    }

    /// Résultats de `generator` pour la ligne `tokens` (le dernier est le mot en
    /// cours). `None` tant qu'ils sont en cours de calcul.
    pub fn lookup(
        &self,
        module: &str,
        generator: &Generator,
        cwd: &Path,
        tokens: &[String],
    ) -> Option<Arc<Vec<Item>>> {
        let (Some(worker), Some((current, before))) = (&self.worker, tokens.split_last()) else {
            return Some(Arc::default());
        };
        let mut context = before.to_vec();
        // Avec un `trigger`, le mot en cours compte jusqu'à sa dernière occurrence.
        if let Some(trigger) = generator.trigger.as_deref() {
            if let Some(i) = current.rfind(trigger) {
                context.push(current[..i + trigger.len()].to_string());
            }
        }
        let key = Key {
            module: module.to_string(),
            path: generator.path.clone(),
            cwd: cwd.to_path_buf(),
            context,
        };

        let mut cache = self.cache.lock().unwrap();
        if cache.len() > MAX_CACHE_ENTRIES {
            cache.retain(|_, entry| entry.running);
        }
        let entry = cache.entry(key.clone()).or_default();
        let fresh = entry.fetched.is_some_and(|t| t.elapsed() < FRESH_FOR);
        if !fresh && !entry.running {
            entry.running = true;
            let job = Job::Generator {
                key,
                tokens: tokens.to_vec(),
            };
            if worker.send(job).is_err() {
                entry.running = false;
            }
        }
        entry.items.clone()
    }

    /// `base` complétée par la fonction `generateSpec` au chemin `path` du
    /// module, lancée dans `cwd` avec les mots `tokens`. Le résultat est gardé
    /// une minute par dossier ; ensuite il est recalculé en arrière-plan.
    pub(crate) fn spec(
        &self,
        module: &str,
        path: &[PathKey],
        base: &Command,
        cwd: &Path,
        tokens: &[String],
    ) -> Generated {
        let Some(worker) = &self.worker else {
            return Generated::Ready(None);
        };
        let key = SpecKey {
            module: module.to_string(),
            path: path.to_vec(),
            cwd: cwd.to_path_buf(),
        };
        let mut specs = self.specs.lock().unwrap();
        if specs.len() >= MAX_SPEC_ENTRIES && !specs.contains_key(&key) {
            specs.retain(|_, entry| entry.running);
        }
        let entry = specs.entry(key.clone()).or_default();
        let fresh = entry.fetched.is_some_and(|t| t.elapsed() < SPEC_FRESH_FOR);
        if !fresh && !entry.running {
            entry.running = true;
            let job = Job::Spec {
                key,
                tokens: tokens.to_vec(),
            };
            if worker.send(job).is_err() {
                entry.running = false;
            }
        }
        match &entry.generated {
            None => Generated::Pending,
            Some(None) => Generated::Ready(None),
            Some(Some(generated)) => {
                let merged = entry.merged.get_or_insert_with(|| {
                    let mut merged = base.clone();
                    merged.merge(generated);
                    Arc::new(merged)
                });
                Generated::Ready(Some(Arc::clone(merged)))
            }
        }
    }
}

/// Moteur JS : les modules des specs y sont chargés à la demande.
struct Engine {
    context: Context,
    sources: HashMap<String, String>,
    loaded: HashSet<String>,
    deadline: Arc<Mutex<Instant>>,
    env: HashMap<String, String>,
}

impl Engine {
    fn new(sources: HashMap<String, String>) -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(64 * 1024 * 1024);
        runtime.set_max_stack_size(1024 * 1024);
        let deadline = Arc::new(Mutex::new(Instant::now() + RUN_TIMEOUT));
        let interrupt = Arc::clone(&deadline);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            Instant::now() > *interrupt.lock().unwrap()
        })));
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        context.with(|ctx| -> Result<(), String> {
            let exec = Function::new(
                ctx.clone(),
                |command: String, args: Vec<String>, cwd: String| {
                    let output = exec::run(&command, &args, Path::new(&cwd), COMMAND_TIMEOUT);
                    serde_json::to_string(&output).unwrap_or_default()
                },
            )
            .map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__easytab_exec", exec)
                .map_err(|e| e.to_string())?;
            for script in [
                include_str!("fig-convert.js"),
                include_str!("generators.js"),
            ] {
                ctx.eval::<(), _>(script)
                    .catch(&ctx)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        })?;
        Ok(Self {
            context,
            sources,
            loaded: HashSet::new(),
            deadline,
            env: std::env::vars().collect(),
        })
    }

    fn run(
        &mut self,
        module: &str,
        path: &[PathKey],
        tokens: &[String],
        cwd: &Path,
    ) -> Result<Vec<Item>, String> {
        *self.deadline.lock().unwrap() = Instant::now() + RUN_TIMEOUT;
        self.load(module)?;
        let path = serde_json::to_string(path).map_err(|e| e.to_string())?;
        let cwd = cwd.to_string_lossy().into_owned();
        let json = self.context.with(|ctx| -> Result<String, String> {
            let env = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
            for (name, value) in &self.env {
                env.set(name.as_str(), value.as_str())
                    .map_err(|e| e.to_string())?;
            }
            let run: Function = ctx
                .globals()
                .get("__easytab_run")
                .map_err(|e| e.to_string())?;
            let promise: Promise = run
                .call((module, path, tokens.to_vec(), cwd, env))
                .catch(&ctx)
                .map_err(|e| e.to_string())?;
            promise
                .finish::<String>()
                .catch(&ctx)
                .map_err(|e| e.to_string())
        })?;
        let mut items: Vec<Item> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        items.sort_by_key(|item| -item.priority);
        Ok(items)
    }

    /// Lance `generateSpec` et convertit la spec produite.
    fn generate(&mut self, key: &SpecKey, tokens: &[String]) -> Result<Option<Command>, String> {
        *self.deadline.lock().unwrap() = Instant::now() + RUN_TIMEOUT;
        self.load(&key.module)?;
        let path = serde_json::to_string(&key.path).map_err(|e| e.to_string())?;
        let cwd = key.cwd.to_string_lossy().into_owned();
        let json = self.context.with(|ctx| -> Result<String, String> {
            let generate: Function = ctx
                .globals()
                .get("__easytab_generate")
                .map_err(|e| e.to_string())?;
            let promise: Promise = generate
                .call((
                    key.module.as_str(),
                    path,
                    key.generated_module(),
                    tokens.to_vec(),
                    cwd,
                ))
                .catch(&ctx)
                .map_err(|e| e.to_string())?;
            promise
                .finish::<String>()
                .catch(&ctx)
                .map_err(|e| e.to_string())
        })?;
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }

    fn load(&mut self, module: &str) -> Result<(), String> {
        // Une spec générée est déjà dans le moteur (ou en est sortie) : elle
        // n'a pas de source.
        if self.loaded.contains(module) || module.contains(GENERATED) {
            return Ok(());
        }
        let source = self
            .sources
            .get(module)
            .ok_or_else(|| format!("module inconnu : {module}"))?;
        self.context.with(|ctx| -> Result<(), String> {
            let (evaluated, promise) = Module::declare(ctx.clone(), module, source.as_str())
                .catch(&ctx)
                .map_err(|e| e.to_string())?
                .eval()
                .catch(&ctx)
                .map_err(|e| e.to_string())?;
            promise
                .finish::<()>()
                .catch(&ctx)
                .map_err(|e| e.to_string())?;
            let exported: Value = evaluated.get("default").map_err(|e| e.to_string())?;
            let registry: Object = ctx
                .globals()
                .get("__easytab_modules")
                .map_err(|e| e.to_string())?;
            registry.set(module, exported).map_err(|e| e.to_string())
        })?;
        self.loaded.insert(module.to_string());
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const MODULE: &str = r#"
        const gen = {
          script: ["sh", "-c", "printf 'main\nfeature/x\n'"],
          postProcess: (out, tokens) =>
            out.split("\n").map((name) => ({ name, description: tokens.join(" ") })),
        };
        export default {
          name: "demo",
          args: [{ generators: [gen, {
            custom: async (tokens, exec) => {
              const { stdout } = await exec({ command: "sh", args: ["-c", "echo a; echo b"] });
              return stdout.split("\n").map((name, i) => ({ name, priority: 50 + i }));
            },
          }, { script: "echo un; echo deux", splitOn: "\n" }] }],
        };
    "#;

    fn generator(i: usize) -> Generator {
        Generator {
            path: vec![
                PathKey::Key("args".into()),
                PathKey::Index(0),
                PathKey::Key("generators".into()),
                PathKey::Index(i),
            ],
            ..Generator::default()
        }
    }

    fn engine() -> Engine {
        Engine::new(HashMap::from([("demo".to_string(), MODULE.to_string())])).unwrap()
    }

    fn tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[cfg(unix)]
    fn names(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.names[0].as_str()).collect()
    }

    #[cfg(unix)]
    #[test]
    fn runs_script_generators() {
        let items = engine()
            .run(
                "demo",
                &generator(0).path,
                &tokens(&["demo", "ma"]),
                Path::new("/"),
            )
            .unwrap();
        assert_eq!(names(&items), ["main", "feature/x"]);
        assert_eq!(items[0].description.as_deref(), Some("demo ma"));
    }

    #[cfg(unix)]
    #[test]
    fn runs_custom_generators_and_sorts_by_priority() {
        let items = engine()
            .run(
                "demo",
                &generator(1).path,
                &tokens(&["demo", ""]),
                Path::new("/"),
            )
            .unwrap();
        assert_eq!(names(&items), ["b", "a"]);
    }

    #[cfg(unix)]
    #[test]
    fn runs_shell_scripts_with_split_on() {
        let items = engine()
            .run(
                "demo",
                &generator(2).path,
                &tokens(&["demo", ""]),
                Path::new("/"),
            )
            .unwrap();
        assert_eq!(names(&items), ["un", "deux"]);
    }

    #[test]
    fn reports_javascript_errors() {
        let mut engine = engine();
        assert!(engine
            .run(
                "demo",
                &generator(7).path,
                &tokens(&["demo", ""]),
                Path::new("/")
            )
            .is_err());
        assert!(engine
            .run("absent", &generator(0).path, &tokens(&[""]), Path::new("/"))
            .is_err());
    }

    #[test]
    fn stops_endless_scripts() {
        let engine = engine();
        *engine.deadline.lock().unwrap() = Instant::now() + Duration::from_millis(100);
        let start = Instant::now();
        let result = engine.context.with(|ctx| {
            ctx.eval::<(), _>("for (;;) {}")
                .catch(&ctx)
                .map_err(|e| e.to_string())
        });
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn caches_results_and_notifies() {
        let (ready, notified) = mpsc::channel();
        let generators = Generators::start_with(
            HashMap::from([("demo".to_string(), MODULE.to_string())]),
            move || {
                let _ = ready.send(());
            },
        );
        let line = tokens(&["demo", "ma"]);
        assert!(generators
            .lookup("demo", &generator(0), Path::new("/"), &line)
            .is_none());
        notified.recv_timeout(Duration::from_secs(5)).unwrap();
        let items = generators
            .lookup("demo", &generator(0), Path::new("/"), &line)
            .unwrap();
        assert_eq!(names(&items), ["main", "feature/x"]);
        // Le mot en cours ne fait pas partie de la clé : pas de nouveau calcul.
        let other = tokens(&["demo", "feat"]);
        assert!(generators
            .lookup("demo", &generator(0), Path::new("/"), &other)
            .is_some());
    }

    #[test]
    fn disabled_generators_give_nothing() {
        let generators = Generators::disabled();
        let items = generators
            .lookup(
                "demo",
                &generator(0),
                Path::new("/"),
                &tokens(&["demo", ""]),
            )
            .unwrap();
        assert!(items.is_empty());
    }

    /// Spec dont `generateSpec` lit le dossier courant et ajoute une
    /// sous-commande avec un generator, et en remplace une autre.
    #[cfg(unix)]
    pub(crate) const GENERATED_MODULE: &str = r#"
        export default {
          name: "gen",
          subcommands: [{ name: "statique" }, { name: "remplacee", description: "avant" }],
          generateSpec: async (tokens, executeShellCommand) => {
            const { stdout } = await executeShellCommand({ command: "pwd", args: [] });
            return {
              name: "gen",
              subcommands: [
                { name: "remplacee", description: "après" },
                {
                  name: "dynamique",
                  description: stdout,
                  loadSpec: "gen/charge",
                  args: { generators: { custom: async () => [{ name: "valeur-" + tokens.length }] } },
                },
              ],
              options: [{ name: "--genere" }],
            };
          },
        };
    "#;

    fn spec_key() -> SpecKey {
        SpecKey {
            module: "gen".into(),
            path: Vec::new(),
            cwd: PathBuf::from("/"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn generates_specs_whose_generators_still_run() {
        let mut engine =
            Engine::new(HashMap::from([("gen".into(), GENERATED_MODULE.into())])).unwrap();
        let key = spec_key();
        let spec = engine
            .generate(&key, &tokens(&["gen", ""]))
            .unwrap()
            .unwrap();
        let subs: Vec<_> = spec.subcommands.iter().map(|s| &s.names[0]).collect();
        assert_eq!(subs, ["remplacee", "dynamique"]);
        let dynamic = &spec.subcommands[1];
        assert_eq!(dynamic.description.as_deref(), Some("/"));
        assert_eq!(dynamic.load.as_deref(), Some("gen/charge"));
        // Pas de `generate` dans une spec déjà générée.
        assert!(spec.generate.is_none());

        // Le generator se lit dans l'objet produit, rangé sous son propre nom.
        let generator = &dynamic.args[0].generators[0];
        let module = generator.module.as_deref().unwrap();
        assert_eq!(module, key.generated_module());
        let items = engine
            .run(
                module,
                &generator.path,
                &tokens(&["gen", "x", ""]),
                Path::new("/"),
            )
            .unwrap();
        assert_eq!(names(&items), ["valeur-2"]);
    }

    #[cfg(unix)]
    #[test]
    fn merges_generated_specs_in_the_background() {
        let (ready, notified) = mpsc::channel();
        let generators = Generators::start_with(
            HashMap::from([("gen".into(), GENERATED_MODULE.into())]),
            move || {
                let _ = ready.send(());
            },
        );
        let base: Command = serde_json::from_str(
            r#"{"names": ["gen"], "subcommands": [{"names": ["statique"]},
                {"names": ["remplacee"], "description": "avant"}]}"#,
        )
        .unwrap();
        let line = tokens(&["gen", ""]);
        let lookup = || generators.spec("gen", &[], &base, Path::new("/"), &line);
        assert!(matches!(lookup(), Generated::Pending));
        notified.recv_timeout(Duration::from_secs(5)).unwrap();
        let Generated::Ready(Some(merged)) = lookup() else {
            panic!("spec générée absente");
        };
        let subs: Vec<_> = merged
            .subcommands
            .iter()
            .map(|s| (s.names[0].as_str(), s.description.as_deref()))
            .collect();
        assert_eq!(
            subs,
            [
                ("statique", None),
                ("remplacee", Some("après")),
                ("dynamique", Some("/"))
            ]
        );
        assert_eq!(merged.options[0].names, ["--genere"]);
        // Gardée : pas de nouveau calcul, la même spec fusionnée.
        let Generated::Ready(Some(again)) = lookup() else {
            panic!("spec générée oubliée");
        };
        assert!(Arc::ptr_eq(&merged, &again));
    }

    #[test]
    fn failing_generate_spec_gives_the_static_spec() {
        let mut engine = Engine::new(HashMap::from([(
            "gen".into(),
            "export default { generateSpec: async () => { throw new Error('non'); } };".into(),
        )]))
        .unwrap();
        assert!(engine.generate(&spec_key(), &tokens(&["gen", ""])).is_err());
        assert!(matches!(
            Generators::disabled().spec("gen", &[], &Command::default(), Path::new("/"), &[]),
            Generated::Ready(None)
        ));
    }
}
