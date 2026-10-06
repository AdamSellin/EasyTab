//! Commandes sans spec : les complétions que le shell connaît déjà.
//!
//! fish (`complete -C`, avec descriptions) et bash-completion (fonction `-F`
//! de la commande) savent compléter beaucoup de commandes que les specs ne
//! décrivent pas. Ils sont interrogés en arrière-plan, un seul à la fois et
//! toujours pour la dernière ligne demandée ; les réponses sont gardées par
//! dossier et par ligne. Une commande qu'aucun ne connaît est notée : le
//! `--help` prend alors le relais.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::exec;
use crate::line::Token;

/// Durée maximale d'une interrogation.
const TIMEOUT: Duration = Duration::from_secs(2);
/// Au-delà, les réponses gardées sont oubliées.
const MAX_CACHE_ENTRIES: usize = 256;
/// Code de sortie des scripts quand le shell ne connaît pas la commande.
const UNKNOWN: i32 = 4;

/// Charge bash-completion, puis appelle la fonction de complétion de la
/// commande comme le ferait bash après Tab. Les mots arrivent en arguments,
/// le dernier étant le mot en cours.
const BASH_SCRIPT: &str = r#"
for f in /usr/share/bash-completion/bash_completion \
    /opt/homebrew/share/bash-completion/bash_completion \
    /usr/local/share/bash-completion/bash_completion \
    /opt/homebrew/etc/profile.d/bash_completion.sh \
    /usr/local/etc/profile.d/bash_completion.sh /etc/bash_completion; do
  [[ -r $f ]] && { . "$f"; break; }
done >/dev/null 2>&1
cmd=$1
COMP_WORDS=("$@"); COMP_CWORD=$(($# - 1)); COMP_LINE="$*"; COMP_POINT=${#COMP_LINE}
COMP_TYPE=9; COMP_KEY=9
spec=$(complete -p -- "$cmd" 2>/dev/null)
if [[ -z $spec ]]; then
  { _comp_load -- "$cmd" || __load_completion "$cmd" || _completion_loader "$cmd"; } >/dev/null 2>&1
  spec=$(complete -p -- "$cmd" 2>/dev/null)
fi
[[ $spec =~ -F\ ([^ ]+) ]] || exit 4
func=${BASH_REMATCH[1]}
[[ $func == _minimal || $func == _comp_complete_minimal ]] && exit 4
COMPREPLY=()
"$func" "$cmd" "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD-1]}" >/dev/null 2>&1
printf '%s\n' "${COMPREPLY[@]}"
"#;

/// `complete -C` sur la ligne (premier argument) ; la commande (second
/// argument) doit avoir ses propres complétions, sinon fish ne propose que
/// des fichiers. Sans la config de l'utilisateur, pour aller vite, mais avec
/// ses complétions.
const FISH_SCRIPT: &str = "set -g fish_complete_path ~/.config/fish/completions \
    $__fish_vendor_completionsdirs $__fish_data_dir/completions; \
    set -l out (complete -C -- $argv[1]); \
    test (count (complete -c $argv[2])) -gt 0; or exit 4; \
    printf '%s\\n' $out";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub value: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    Ready(Vec<Item>),
    /// La réponse est en cours de calcul ; son arrivée déclenche `notify`.
    Pending,
    /// Aucun shell ne connaît cette commande.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Fish(PathBuf),
    Bash(PathBuf),
}

/// Ligne à compléter : dossier, mots tels que tapés, et sans guillemets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Request {
    cwd: PathBuf,
    raw: Vec<String>,
    values: Vec<String>,
}

#[derive(Default)]
struct State {
    results: HashMap<Request, Vec<Item>>,
    unknown: HashSet<String>,
    /// Prochaine ligne à compléter (les précédentes sont abandonnées).
    next: Option<Request>,
    /// Ligne en cours de calcul.
    running: Option<Request>,
}

#[derive(Clone)]
pub struct ShellCompletions {
    shared: Arc<(Mutex<State>, Condvar)>,
}

impl ShellCompletions {
    /// Interroge fish s'il est installé, puis le bash `bash` (celui de
    /// l'utilisateur, ou à défaut celui du PATH hors Windows, où `bash` peut
    /// être celui de WSL). `None` s'il n'y a ni l'un ni l'autre.
    pub fn start(bash: Option<&str>, notify: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let mut sources = Vec::new();
        if !cfg!(windows) {
            if let Some(fish) = exec::find_in_path("fish") {
                sources.push(Source::Fish(fish));
            }
        }
        let bash = match bash {
            Some(bash) if bash.contains(['/', '\\']) => Some(PathBuf::from(bash)),
            Some(bash) => exec::find_in_path(bash),
            None if !cfg!(windows) => exec::find_in_path("bash"),
            None => None,
        };
        sources.extend(bash.map(Source::Bash));
        if sources.is_empty() {
            return None;
        }
        Some(Self::with_sources(sources, notify))
    }

    fn with_sources(sources: Vec<Source>, notify: impl Fn() + Send + Sync + 'static) -> Self {
        let shared: Arc<(Mutex<State>, Condvar)> = Arc::default();
        let worker = Arc::clone(&shared);
        thread::spawn(move || loop {
            let request = {
                let (lock, ready) = &*worker;
                let mut state = lock.lock().unwrap();
                loop {
                    if let Some(request) = state.next.take() {
                        state.running = Some(request.clone());
                        break request;
                    }
                    state = ready.wait(state).unwrap();
                }
            };
            let found = sources.iter().find_map(|source| ask(source, &request));
            let (lock, _) = &*worker;
            let mut state = lock.lock().unwrap();
            state.running = None;
            match found {
                Some(items) => {
                    if state.results.len() >= MAX_CACHE_ENTRIES {
                        state.results.clear();
                    }
                    state.results.insert(request, items);
                }
                None => {
                    state.unknown.insert(request.values[0].clone());
                }
            }
            drop(state);
            notify();
        });
        Self { shared }
    }

    /// Complétions du mot `current` après les mots `words` (commande
    /// comprise), dans le dossier `cwd`.
    pub fn lookup(&self, words: &[Token], current: &Token, cwd: &Path) -> Lookup {
        let Some(command) = words.first() else {
            return Lookup::Unknown;
        };
        let request = Request {
            cwd: cwd.to_path_buf(),
            raw: words
                .iter()
                .chain([current])
                .map(|t| t.raw.clone())
                .collect(),
            values: words
                .iter()
                .chain([current])
                .map(|t| t.value.clone())
                .collect(),
        };
        let (lock, ready) = &*self.shared;
        let mut state = lock.lock().unwrap();
        if state.unknown.contains(&command.value) {
            return Lookup::Unknown;
        }
        if let Some(items) = state.results.get(&request) {
            return Lookup::Ready(items.clone());
        }
        if state.running.as_ref() != Some(&request) {
            state.next = Some(request);
            ready.notify_one();
        }
        Lookup::Pending
    }
}

/// Interroge un shell ; `None` s'il ne connaît pas la commande.
fn ask(source: &Source, request: &Request) -> Option<Vec<Item>> {
    let (program, args) = match source {
        Source::Fish(fish) => {
            // `complete -C` relit la ligne : on garde les mots tels que tapés.
            let line = request.raw.join(" ");
            let args = vec![
                "--no-config".to_string(),
                "-c".to_string(),
                FISH_SCRIPT.to_string(),
                line,
                request.values[0].clone(),
            ];
            (fish, args)
        }
        Source::Bash(bash) => {
            let mut args = vec![
                "--norc".to_string(),
                "--noprofile".to_string(),
                "-c".to_string(),
                BASH_SCRIPT.to_string(),
                "easytab".to_string(),
            ];
            args.extend(request.values.iter().cloned());
            (bash, args)
        }
    };
    let output = exec::run(&program.to_string_lossy(), &args, &request.cwd, TIMEOUT);
    match output.status {
        0 => Some(parse(&output.stdout)),
        // Commande inconnue de ce shell, ou shell impossible à lancer.
        UNKNOWN | 126 | 127 => None,
        // Trop long : rien pour cette ligne, sans renoncer à la commande.
        _ => Some(Vec::new()),
    }
}

/// Une valeur par ligne, suivie d'une tabulation et de sa description (fish).
fn parse(stdout: &str) -> Vec<Item> {
    let mut seen = HashSet::new();
    stdout
        .lines()
        .filter_map(|line| {
            let (value, description) = match line.split_once('\t') {
                Some((value, description)) => (value, Some(description.trim())),
                None => (line, None),
            };
            let value = value.trim_end();
            (!value.is_empty() && seen.insert(value.to_string())).then(|| Item {
                value: value.to_string(),
                description: description.filter(|d| !d.is_empty()).map(str::to_string),
            })
        })
        .collect()
}

/// bash dont les complétions sont déclarées par `definitions` (code bash,
/// sans apostrophe) plutôt que par bash-completion, qui n'est pas toujours
/// installé : un faux `bash`, dans `dir`, les ajoute au script d'EasyTab.
#[cfg(all(test, unix))]
pub(crate) fn fake_bash(dir: &Path, definitions: &str) -> Option<ShellCompletions> {
    use std::os::unix::fs::PermissionsExt;
    let real_bash = exec::find_in_path("bash")?;
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let wrapper = dir.join("bash");
    // Reçoit `--norc --noprofile -c SCRIPT easytab mots…`.
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nscript=$4\nshift 4\nexec {} --norc --noprofile -c '{definitions}; '\"$script\" \"$@\"\n",
            real_bash.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    Some(ShellCompletions::with_sources(
        vec![Source::Bash(wrapper)],
        || {},
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::line;
    #[cfg(unix)]
    use std::time::Instant;

    #[test]
    fn parses_values_and_descriptions() {
        assert_eq!(
            parse("install\tInstall one or more packages\nremove\n\nremove\n"),
            [
                Item {
                    value: "install".into(),
                    description: Some("Install one or more packages".into())
                },
                Item {
                    value: "remove".into(),
                    description: None
                },
            ]
        );
    }

    #[cfg(unix)]
    fn wait(shell: &ShellCompletions, input: &str, cwd: &Path) -> Lookup {
        let line = line::parse(input);
        let start = Instant::now();
        loop {
            match shell.lookup(&line.words, &line.current, cwd) {
                Lookup::Pending if start.elapsed() < Duration::from_secs(10) => {
                    thread::sleep(Duration::from_millis(20))
                }
                other => return other,
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn asks_bash_for_its_completions() {
        let dir = std::env::temp_dir().join(format!("easytab-shell-{}", std::process::id()));
        let Some(shell) = fake_bash(
            &dir,
            "_outil() { COMPREPLY=($(compgen -W \"deploy destroy status\" -- \"$2\")); }; \
             complete -F _outil outil",
        ) else {
            return;
        };
        let values = |lookup: Lookup| match lookup {
            Lookup::Ready(items) => items.into_iter().map(|i| i.value).collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            values(wait(&shell, "outil de", &dir)),
            ["deploy", "destroy"]
        );
        assert_eq!(wait(&shell, "inconnu x", &dir), Lookup::Unknown);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
