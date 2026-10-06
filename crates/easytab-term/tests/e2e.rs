//! Test de bout en bout : lance `easytab-term` dans un vrai pseudo-terminal
//! (ConPTY sous Windows), avec PowerShell ou bash, tape une commande, choisit
//! une suggestion dans la liste et vérifie la ligne obtenue.
//!
//! Il couvre ce que les tests unitaires ne voient pas : le démarrage du shell
//! sous le wrapper, l'affichage de la liste, et les touches Entrée et Échap
//! telles que la console les envoie.

#![cfg(any(windows, target_os = "linux"))]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

const ROWS: u16 = 30;
const COLS: u16 = 100;
const TIMEOUT: Duration = Duration::from_secs(60);

/// Le wrapper qui tourne dans un pseudo-terminal, et une copie de l'écran.
struct Terminal {
    writer: Box<dyn Write + Send>,
    output: mpsc::Receiver<Vec<u8>>,
    parser: vt100::Parser,
    /// Journal d'easytab-term (`EASYTAB_LOG`), montré en cas d'échec.
    log: PathBuf,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
}

impl Terminal {
    fn start(shell: &str, args: &[String], home: &Path) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("ouverture du pseudo-terminal");
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_easytab-term"));
        cmd.arg("--shell");
        cmd.arg(shell);
        cmd.arg("--");
        cmd.args(args);
        cmd.cwd(home);
        cmd.env("HOME", home);
        cmd.env("USERPROFILE", home);
        // L'historique de PowerShell (PSReadLine) est dans %APPDATA%.
        cmd.env("APPDATA", home.join("AppData").join("Roaming"));
        cmd.env("TERM", "xterm-256color");
        // La liste dans le terminal : la fenêtre flottante n'a pas d'écran ici.
        cmd.env("EASYTAB_OVERLAY", "0");
        let log = home.join("easytab.log");
        cmd.env("EASYTAB_LOG", &log);
        cmd.env_remove("EASYTAB_TERM");
        cmd.env_remove("EASYTAB_DISABLE");
        let child = pair
            .slave
            .spawn_command(cmd)
            .expect("lancement d'easytab-term");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let writer = pair.master.take_writer().unwrap();
        Self {
            writer,
            output,
            parser: vt100::Parser::new(ROWS, COLS, 0),
            log,
            _child: child,
            _master: pair.master,
        }
    }

    /// Lit ce qui arrive pendant `duration`, en répondant aux demandes de
    /// position du curseur (`ESC[6n`) comme le ferait un vrai terminal.
    fn pump(&mut self, duration: Duration) {
        let end = Instant::now() + duration;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            let Ok(data) = self
                .output
                .recv_timeout(left.min(Duration::from_millis(50)))
            else {
                continue;
            };
            let mut rest = &data[..];
            while let Some(at) = find(rest, b"\x1b[6n") {
                self.parser.process(&rest[..at]);
                let (row, col) = self.parser.screen().cursor_position();
                self.send(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
                rest = &rest[at + 4..];
            }
            self.parser.process(rest);
        }
    }

    fn send(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).unwrap();
        self.writer.flush().unwrap();
    }

    fn screen(&self) -> String {
        self.parser.screen().contents()
    }

    /// La ligne où se trouve le curseur, jusqu'au curseur : la suggestion en
    /// gris, après lui, n'en fait pas partie.
    fn cursor_line(&self) -> String {
        let (row, col) = self.parser.screen().cursor_position();
        self.parser
            .screen()
            .rows(0, col)
            .nth(row as usize)
            .unwrap_or_default()
            .trim_end()
            .to_string()
    }

    /// L'écran et la fin du journal d'easytab-term, pour comprendre un échec.
    fn report(&self) -> String {
        let log = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        let tail = lines[lines.len().saturating_sub(40)..].join("\n");
        format!("Écran :\n{}\nJournal :\n{tail}", self.screen())
    }

    /// Attend que l'écran vérifie `ok`, sinon échoue en montrant l'écran.
    fn wait_for(&mut self, what: &str, ok: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        while !ok(self) {
            if start.elapsed() > TIMEOUT {
                panic!(
                    "{what} : toujours absent après {TIMEOUT:?}.\n{}",
                    self.report()
                );
            }
            self.pump(Duration::from_millis(200));
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Dossier temporaire propre au test.
fn temp_home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("easytab-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Script d'intégration du dépôt, tel que `easytab init` le génère.
fn integration(file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../shell-integration")
        .join(file);
    std::fs::read_to_string(path)
        .unwrap()
        .replace("__EASYTAB_TERM_BIN__", "'easytab-term-absent'")
}

/// Tape `git checko`, attend la liste, choisit `checkout` avec Tab : la ligne
/// doit devenir `git checkout `. Puis, dans la liste des options, ↓ et Entrée
/// choisissent une option sans lancer la commande, et Échap ferme la liste
/// sans rien insérer.
fn completes_git_checkout(term: &mut Terminal) {
    term.send(b"git checko");
    // La description de `checkout` s'affiche sous la liste. Le shell doit
    // aussi avoir tout affiché : PowerShell peut n'avoir montré que `git c`,
    // dont la liste décrit déjà `checkout`.
    term.wait_for("la liste avec checkout", |t| {
        t.screen().contains("Switch branches") && t.cursor_line().ends_with("git checko")
    });
    term.send(b"\t");
    term.wait_for("la ligne git checkout", |t| {
        t.cursor_line().ends_with("git checkout") && !t.screen().contains("Switch branches")
    });
    // Options : ↓ puis Entrée insère la 2e option, sans lancer la commande.
    term.send(b"--");
    term.wait_for("la liste des options", |t| {
        t.screen().contains("--conflict")
    });
    term.send(b"\x1b[B");
    term.pump(Duration::from_millis(300));
    term.send(b"\r");
    term.wait_for("une option insérée", |t| {
        let line = t.cursor_line();
        line.contains("git checkout --") && !line.ends_with("git checkout --")
    });
    assert!(
        !term.screen().contains("not a git command") && !term.screen().contains("usage: git"),
        "Entrée dans la liste ne doit pas lancer la commande.\n{}",
        term.report()
    );
    // Échap ferme la liste sans rien insérer. L'option choisie peut finir par
    // `=` (`--conflict=`) : on repart d'une nouvelle option, ` -`, pour que la
    // liste des options s'ouvre à coup sûr.
    term.send(b" -");
    term.wait_for("la liste après ` -`", |t| {
        t.cursor_line().ends_with(" -") && t.screen().contains('╭')
    });
    let line = term.cursor_line();
    term.send(b"\x1b");
    term.wait_for("la liste fermée", |t| !t.screen().contains('╭'));
    term.pump(Duration::from_millis(300));
    assert_eq!(
        term.cursor_line(),
        line,
        "Échap ne doit rien insérer.\n{}",
        term.report()
    );
}

#[cfg(windows)]
#[test]
fn powershell_suggests_and_inserts() {
    let home = temp_home("pwsh");
    let script = home.join("easytab.ps1");
    std::fs::write(&script, integration("easytab.ps1")).unwrap();
    // PowerShell 7 si présent, sinon Windows PowerShell 5.1.
    let shell = if which("pwsh.exe") {
        "pwsh.exe"
    } else {
        "powershell.exe"
    };
    let args = [
        "-NoLogo".to_string(),
        "-NoProfile".to_string(),
        "-NoExit".to_string(),
        "-Command".to_string(),
        format!(". '{}'", script.display()),
    ];
    let mut term = Terminal::start(shell, &args, &home);
    term.wait_for("le prompt PowerShell", |t| t.cursor_line().ends_with('>'));
    completes_git_checkout(&mut term);
}

/// Les paramètres des commandes PowerShell viennent de PowerShell lui-même.
#[cfg(windows)]
#[test]
fn powershell_suggests_cmdlet_parameters() {
    let home = temp_home("pwsh-params");
    let script = home.join("easytab.ps1");
    std::fs::write(&script, integration("easytab.ps1")).unwrap();
    let shell = if which("pwsh.exe") {
        "pwsh.exe"
    } else {
        "powershell.exe"
    };
    let args = [
        "-NoLogo".to_string(),
        "-NoProfile".to_string(),
        "-NoExit".to_string(),
        "-Command".to_string(),
        format!(". '{}'", script.display()),
    ];
    let mut term = Terminal::start(shell, &args, &home);
    term.wait_for("le prompt PowerShell", |t| t.cursor_line().ends_with('>'));
    term.send(b"Remove-Item -Recu");
    term.wait_for("le paramètre -Recurse", |t| {
        t.screen().contains("-Recurse") && !t.cursor_line().ends_with("-Recurse")
    });
    term.send(b"\t");
    term.wait_for("la ligne Remove-Item -Recurse", |t| {
        t.cursor_line().ends_with("Remove-Item -Recurse")
    });
}

/// PowerShell sous `easytab-term`, prompt affiché.
#[cfg(windows)]
fn start_powershell(name: &str) -> Terminal {
    let home = temp_home(name);
    let script = home.join("easytab.ps1");
    std::fs::write(&script, integration("easytab.ps1")).unwrap();
    let shell = if which("pwsh.exe") {
        "pwsh.exe"
    } else {
        "powershell.exe"
    };
    let args = [
        "-NoLogo".to_string(),
        "-NoProfile".to_string(),
        "-NoExit".to_string(),
        "-Command".to_string(),
        format!(". '{}'", script.display()),
    ];
    let mut term = Terminal::start(shell, &args, &home);
    term.wait_for("le prompt PowerShell", |t| t.cursor_line().ends_with('>'));
    term
}

/// Un alias PowerShell reçoit les paramètres de sa cmdlet : `ls` est
/// Get-ChildItem, pas le `ls` d'Unix.
#[cfg(windows)]
#[test]
fn powershell_aliases_get_cmdlet_parameters() {
    let mut term = start_powershell("pwsh-alias");
    term.send(b"ls -Recu");
    term.wait_for("le paramètre -Recurse de ls", |t| {
        t.screen().contains("-Recurse") && !t.cursor_line().ends_with("-Recurse")
    });
    term.send(b"\t");
    term.wait_for("la ligne ls -Recurse", |t| {
        t.cursor_line().ends_with("ls -Recurse")
    });
}

/// Specs des outils Windows : options en `/`, sans tenir compte de la casse.
#[cfg(windows)]
#[test]
fn completes_windows_tools() {
    let mut term = start_powershell("pwsh-windows-tools");
    term.send(b"robocopy a b /mir");
    term.wait_for("l'option /MIR", |t| t.screen().contains("/MIR"));
    term.send(b"\t");
    term.wait_for("la ligne robocopy a b /MIR", |t| {
        t.cursor_line().ends_with("robocopy a b /MIR")
    });
    // La valeur d'une option, dans le mot suivant.
    let mut term = start_powershell("pwsh-windows-tools-values");
    term.send(b"tasklist /fo ");
    term.wait_for("les formats de tasklist", |t| {
        let screen = t.screen();
        screen.contains("TABLE") && screen.contains("CSV")
    });
}

#[cfg(windows)]
fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Les commandes déjà tapées (`~/.bash_history`) sont proposées entières.
#[test]
fn bash_suggests_history() {
    let Some(mut term) = start_bash("bash-history", |home| {
        std::fs::write(home.join(".bash_history"), "echo easytab-history-test\n").unwrap();
    }) else {
        return;
    };
    term.send(b"echo eas");
    term.wait_for("la commande de l'historique", |t| {
        t.screen().contains("echo easytab-history-test")
    });
    term.send(b"\t");
    term.wait_for("la ligne complétée", |t| {
        t.cursor_line() == "$ echo easytab-history-test"
    });
}

/// Entrée prend la suggestion surlignée qui complète le mot, sans flèche,
/// et ne lance pas la commande.
#[test]
fn enter_inserts_the_highlighted_completion() {
    let Some(mut term) = start_bash("bash-enter", |home| {
        std::fs::write(home.join(".bash_history"), "echo easytab-enter-test\n").unwrap();
    }) else {
        return;
    };
    term.send(b"echo eas");
    term.wait_for("la commande de l'historique", |t| {
        t.screen().contains("echo easytab-enter-test") && t.cursor_line() == "$ echo eas"
    });
    term.send(b"\r");
    term.wait_for("la ligne complétée", |t| {
        t.cursor_line() == "$ echo easytab-enter-test"
    });
    term.pump(Duration::from_millis(300));
    assert_eq!(
        term.cursor_line(),
        "$ echo easytab-enter-test",
        "Entrée ne doit pas lancer la commande.\n{}",
        term.report()
    );
}

/// `make ` propose les cibles du Makefile du dossier courant.
#[cfg(target_os = "linux")]
#[test]
fn bash_suggests_make_targets() {
    let Some(mut term) = start_bash("bash-make", |home| {
        std::fs::write(
            home.join("Makefile"),
            "easytab-cible: ## Cible de test\n\ttrue\n",
        )
        .unwrap();
    }) else {
        return;
    };
    term.send(b"make easytab-c");
    term.wait_for("la cible du Makefile", |t| {
        t.screen().contains("Cible de test")
    });
    term.send(b"\t");
    term.wait_for("la ligne complétée", |t| {
        t.cursor_line() == "$ make easytab-cible"
    });
}

/// L'intégration met le dossier de `easytab-term` dans le PATH, une seule
/// fois même chargée deux fois, pour taper `easytab update` sans chemin.
#[test]
fn integration_adds_easytab_to_path() {
    let home = temp_home("path");
    let term = format!("{}/easytab-bin-test/easytab-term", home.display()).replace('\\', "/");
    let script = home.join("easytab.sh");
    let content =
        integration("easytab.bash").replace("'easytab-term-absent'", &format!("'{term}'"));
    std::fs::write(&script, content).unwrap();
    let bash = if cfg!(windows) {
        r"C:\Program Files\Git\bin\bash.exe"
    } else {
        "bash"
    };
    let source = script.display().to_string().replace('\\', "/");
    let Ok(output) = std::process::Command::new(bash)
        .arg("-c")
        .arg(format!(". '{source}'; . '{source}'; echo \"$PATH\""))
        .output()
    else {
        eprintln!("bash absent : test ignoré");
        return;
    };
    let path = String::from_utf8_lossy(&output.stdout);
    assert_eq!(path.matches("easytab-bin-test").count(), 1, "PATH : {path}");
    assert!(path.contains("easytab-bin-test:"), "PATH : {path}");
}

#[cfg(windows)]
#[test]
fn powershell_integration_adds_easytab_to_path() {
    let home = temp_home("pwsh-path");
    let term = home.join("easytab-bin-test").join("easytab-term.exe");
    let script = home.join("easytab.ps1");
    let content = integration("easytab.ps1")
        .replace("'easytab-term-absent'", &format!("'{}'", term.display()));
    std::fs::write(&script, content).unwrap();
    let source = script.display();
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
        ])
        .arg(format!(". '{source}'; . '{source}'; $env:PATH"))
        .output()
        .unwrap();
    let path = String::from_utf8_lossy(&output.stdout);
    assert_eq!(path.matches("easytab-bin-test").count(), 1, "PATH : {path}");
}

/// bash : celui du système sous Linux, Git Bash sous Windows (`None` s'il
/// est absent). `prepare` remplit le dossier personnel avant le lancement.
fn start_bash(name: &str, prepare: impl FnOnce(&Path)) -> Option<Terminal> {
    let bash = if cfg!(windows) {
        let git_bash = Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if !git_bash.is_file() {
            eprintln!("Git Bash absent : test ignoré");
            return None;
        }
        git_bash.display().to_string()
    } else {
        "bash".to_string()
    };
    let home = temp_home(name);
    prepare(&home);
    let rc = home.join("easytabrc");
    std::fs::write(
        &rc,
        format!(
            "PS1='$ '\n[ -f ~/.bash_aliases ] && . ~/.bash_aliases\n{}",
            integration("easytab.bash")
        ),
    )
    .unwrap();
    let args = [
        "--noprofile".to_string(),
        "--rcfile".to_string(),
        rc.display().to_string().replace('\\', "/"),
        "-i".to_string(),
    ];
    let mut term = Terminal::start(&bash, &args, &home);
    term.wait_for("le prompt bash", |t| t.cursor_line() == "$");
    Some(term)
}

#[test]
fn bash_suggests_and_inserts() {
    if let Some(mut term) = start_bash("bash", |_| {}) {
        completes_git_checkout(&mut term);
    }
}

/// Un alias du shell se complète comme sa commande ; Échap ferme la liste,
/// Ctrl+Espace la rouvre, Maj+Tab remonte d'une ligne. `$HOM` propose `$HOME`.
#[test]
fn bash_follows_aliases_and_variables() {
    let Some(mut term) = start_bash("bash-alias", |home| {
        std::fs::write(home.join(".bash_aliases"), "alias g=git\n").unwrap();
    }) else {
        return;
    };
    term.send(b"g checko");
    term.wait_for("la liste de git par l'alias", |t| {
        t.screen().contains("Switch branches") && t.cursor_line() == "$ g checko"
    });
    term.send(b"\x1b");
    term.wait_for("la liste fermée", |t| !t.screen().contains('╭'));
    term.send(b"\0");
    term.wait_for("la liste rouverte", |t| {
        t.screen().contains("Switch branches")
    });
    // Maj+Tab depuis la première ligne passe à la dernière, puis ↓ revient.
    term.send(b"\x1b[Z");
    term.pump(Duration::from_millis(200));
    term.send(b"\x1b[B");
    term.pump(Duration::from_millis(200));
    term.send(b"\t");
    term.wait_for("la ligne g checkout", |t| t.cursor_line() == "$ g checkout");

    term.send(b"\x15echo $HOM");
    term.wait_for("la variable $HOME", |t| {
        t.screen().contains("$HOME") && t.cursor_line() == "$ echo $HOM"
    });
    term.send(b"\t");
    term.wait_for("la ligne complétée", |t| {
        t.cursor_line() == "$ echo $HOME"
    });
}

/// La commande la plus récente de l'historique qui prolonge la ligne
/// s'affiche en gris après le curseur ; → l'accepte.
#[test]
fn right_arrow_accepts_the_inline_suggestion() {
    let Some(mut term) = start_bash("bash-inline", |home| {
        std::fs::write(home.join(".bash_history"), "echo easytab-inline-test\n").unwrap();
    }) else {
        return;
    };
    term.send(b"echo eas");
    term.wait_for("la suggestion en gris", |t| {
        let (row, _) = t.parser.screen().cursor_position();
        let line = t
            .parser
            .screen()
            .rows(0, COLS)
            .nth(row as usize)
            .unwrap_or_default();
        t.cursor_line() == "$ echo eas" && line.trim_end() == "$ echo easytab-inline-test"
    });
    term.send(b"\x1b[C");
    term.wait_for("la ligne complétée", |t| {
        t.cursor_line() == "$ echo easytab-inline-test"
    });
}
