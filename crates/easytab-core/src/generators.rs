//! Generators Fig : suggestions calculées en lançant une commande (branches git,
//! scripts npm, conteneurs docker…) puis en passant sa sortie au JavaScript de
//! la spec.
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
use crate::spec::{Generator, Modules, PathKey};

/// Au-delà, les résultats sont recalculés (en gardant les anciens à l'écran).
const FRESH_FOR: Duration = Duration::from_secs(5);
/// Durée maximale d'une commande lancée par un generator.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// Durée maximale d'un generator, JavaScript compris.
const RUN_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CACHE_ENTRIES: usize = 256;

/// Suggestion produite par un generator.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Item {
    pub names: Vec<String>,
    pub insert: Option<String>,
    pub description: Option<String>,
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

struct Job {
    key: Key,
    tokens: Vec<String>,
}

#[derive(Clone, Default)]
pub struct Generators {
    worker: Option<Sender<Job>>,
    cache: Arc<Mutex<HashMap<Key, Entry>>>,
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
    fn start_with(modules: HashMap<String, String>, notify: impl Fn() + Send + 'static) -> Self {
        Self::spawn(move || modules, notify)
    }

    /// `load` est appelé dans le fil de calcul : lire les modules ne retarde pas
    /// le démarrage du terminal.
    fn spawn(
        load: impl FnOnce() -> HashMap<String, String> + Send + 'static,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let cache: Arc<Mutex<HashMap<Key, Entry>>> = Arc::default();
        let (worker, jobs) = mpsc::channel::<Job>();
        let worker_cache = Arc::clone(&cache);
        thread::Builder::new()
            .name("easytab-generators".into())
            .spawn(move || {
                let mut engine = Engine::new(load());
                for job in jobs {
                    let items = match &mut engine {
                        Ok(engine) => engine
                            .run(&job.key.module, &job.key.path, &job.tokens, &job.key.cwd)
                            .unwrap_or_default(),
                        Err(_) => Vec::new(),
                    };
                    let mut cache = worker_cache.lock().unwrap();
                    let entry = cache.entry(job.key).or_default();
                    entry.items = Some(Arc::new(items));
                    entry.fetched = Some(Instant::now());
                    entry.running = false;
                    drop(cache);
                    notify();
                }
            })
            .expect("lancement du fil des generators");
        Self {
            worker: Some(worker),
            cache,
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
            let job = Job {
                key,
                tokens: tokens.to_vec(),
            };
            if worker.send(job).is_err() {
                entry.running = false;
            }
        }
        entry.items.clone()
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
            ctx.eval::<(), _>(include_str!("generators.js"))
                .catch(&ctx)
                .map_err(|e| e.to_string())
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

    fn load(&mut self, module: &str) -> Result<(), String> {
        if self.loaded.contains(module) {
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
mod tests {
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
}
