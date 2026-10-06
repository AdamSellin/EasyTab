//! Boucle de la fenêtre, commune aux systèmes : une fenêtre sans bordure,
//! transparente, toujours devant, qui ne prend jamais le focus, avec une
//! WebView qui dessine la liste (`popup.html`). Ce qui dépend du système
//! (curseur, écran, terminal au premier plan) est dans `platform`.

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event as WindowEvent, StartCause};
use tao::event_loop::ControlFlow;
use tao::window::{Window, WindowBuilder};
use wry::{WebContext, WebViewBuilder};

use crate::estimate::Hint;
use crate::placement::{place, Caret, Measure, Tracker};
use crate::platform::{self, Locator, Surface};
use crate::protocol::{Event, Request, View};

/// Fréquence à laquelle on relit la position du curseur et vérifie que le
/// terminal est toujours au premier plan.
const WATCH: Duration = Duration::from_millis(150);
/// Délai de la relecture qui suit chaque frappe.
const SETTLE: Duration = Duration::from_millis(50);

pub enum UserEvent {
    Request(Request),
    /// `easytab-term` s'est arrêté.
    Closed,
    /// Message de la page : `"ready"` ou ses mesures.
    Page(String),
    /// Résultat d'une recherche du curseur.
    Caret(u64, Option<Caret>),
}

/// Notre fenêtre et ce qui la montre, la cache et la place.
struct Popup {
    window: Window,
    surface: Surface,
}

pub fn run() -> anyhow::Result<()> {
    // Avant tout fil : `set_var` n'est sûr que dans un programme à un seul fil.
    let data_dir = dirs::data_local_dir().map(|dir| dir.join("EasyTab").join("webview"));
    if let Some(dir) = &data_dir {
        // WebView2 lit aussi cette variable : sans elle, une partie de ses
        // données peut atterrir à côté du programme
        // (`easytab-overlay.exe.WebView2` dans `~/.easytab/bin`).
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", dir);
    }
    remove_stray_webview_data();

    let event_loop = platform::event_loop::<UserEvent>()?;

    let proxy = event_loop.create_proxy();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(request) = serde_json::from_str::<Request>(&line) {
                if proxy.send_event(UserEvent::Request(request)).is_err() {
                    return;
                }
            }
        }
        let _ = proxy.send_event(UserEvent::Closed);
    });

    // La recherche du curseur interroge d'autres programmes (UI Automation,
    // accessibilité, serveur X) : elle a son propre fil pour ne jamais
    // bloquer la fenêtre.
    let (locate, locate_rx) = mpsc::channel::<(u64, Hint)>();
    let proxy = event_loop.create_proxy();
    thread::spawn(move || {
        let locator = Locator::new();
        while let Ok(mut request) = locate_rx.recv() {
            while let Ok(newer) = locate_rx.try_recv() {
                request = newer;
            }
            let (generation, hint) = request;
            if proxy
                .send_event(UserEvent::Caret(generation, locator.locate(hint)))
                .is_err()
            {
                return;
            }
        }
    });

    let builder = WindowBuilder::new()
        .with_title("EasyTab")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top(true)
        .with_visible(false)
        .with_focused(false)
        .with_resizable(false)
        .with_inner_size(LogicalSize::new(320.0, 240.0));
    let window = platform::configure(builder).build(&event_loop)?;
    // Fenêtre jamais active, que les clics traversent.
    let surface = Surface::new(&window);

    let mut context = WebContext::new(data_dir);
    let proxy = event_loop.create_proxy();
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_transparent(true)
        .with_focused(false)
        .with_html(include_str!("popup.html"))
        .with_ipc_handler(move |request| {
            let _ = proxy.send_event(UserEvent::Page(request.into_body()));
        });
    let webview = platform::webview(builder, &window)?;

    let popup = Popup { window, surface };
    let mut state = State::default();
    event_loop.run(move |event, _, control_flow| {
        let _ = &webview;
        match event {
            WindowEvent::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if Instant::now() >= state.next_check {
                    state.watch(&popup, &locate);
                }
            }
            WindowEvent::UserEvent(UserEvent::Request(Request::Show(view))) => {
                let json = serde_json::to_string(&view).unwrap_or_default();
                let _ = webview.evaluate_script(&format!("render({json})"));
                let hint = Hint::from(&view);
                state.view = Some(view);
                // La taille de la page arrive avec le nouveau contenu.
                state.measure = None;
                state.generation += 1;
                state.show_generation = state.generation;
                if !state.shown {
                    state.reads = 0;
                }
                let _ = locate.send((state.generation, hint));
                // Relecture peu après : le terminal met à jour son curseur
                // avec un temps de retard.
                state.next_check = Instant::now() + SETTLE;
            }
            WindowEvent::UserEvent(UserEvent::Request(Request::Hide)) => {
                state.view = None;
                state.generation += 1;
                state.hide(&popup);
            }
            WindowEvent::UserEvent(UserEvent::Page(message)) => {
                if message == "\"ready\"" {
                    send(Event::Ready);
                } else if let Ok(measure) = serde_json::from_str::<Measure>(&message) {
                    state.measure = Some(measure);
                    state.place(&popup);
                }
            }
            WindowEvent::UserEvent(UserEvent::Caret(generation, caret))
                if generation == state.generation && state.view.is_some() =>
            {
                match caret {
                    Some(caret) => {
                        state.failures = 0;
                        state.read(caret, generation, &popup);
                    }
                    // Échec passager : on garde la position connue.
                    None if state.tracker.terminal().is_some() => {}
                    None if state.failures < 2 => state.failures += 1,
                    None => {
                        // Le terminal dessinera la liste lui-même.
                        state.view = None;
                        state.failures = 0;
                        state.hide(&popup);
                        send(Event::Unavailable);
                    }
                }
            }
            WindowEvent::UserEvent(UserEvent::Closed) => {
                *control_flow = ControlFlow::Exit;
                return;
            }
            _ => {}
        }
        *control_flow = if state.view.is_some() {
            ControlFlow::WaitUntil(state.next_check)
        } else {
            ControlFlow::Wait
        };
    });
}

struct State {
    /// Liste demandée par le terminal (`None` : cachée).
    view: Option<View>,
    /// Numéro de la dernière demande : les réponses plus anciennes sont ignorées.
    generation: u64,
    /// Numéro de la lecture demandée par la dernière liste du terminal (la
    /// frappe vient forcément de lui).
    show_generation: u64,
    /// Lectures depuis la première apparition : on en attend deux, la
    /// première est souvent en retard.
    reads: u32,
    tracker: Tracker,
    measure: Option<Measure>,
    /// La fenêtre est à l'écran.
    shown: bool,
    /// Le terminal n'est plus au premier plan ou n'a plus le focus (autre
    /// onglet, autre terminal de VS Code) : la fenêtre est cachée en
    /// attendant qu'il revienne.
    hidden_away: bool,
    /// Lectures du curseur ratées avant d'avoir une position.
    failures: u32,
    /// Prochaine relecture du curseur.
    next_check: Instant,
}

impl Default for State {
    fn default() -> Self {
        Self {
            view: None,
            generation: 0,
            show_generation: 0,
            reads: 0,
            tracker: Tracker::default(),
            measure: None,
            shown: false,
            hidden_away: false,
            failures: 0,
            next_check: Instant::now(),
        }
    }
}

impl State {
    /// Affiche la fenêtre à la position déduite du curseur, quand la taille
    /// de la page est connue.
    fn place(&mut self, popup: &Popup) {
        let (Some(view), Some(measure)) = (&self.view, self.measure) else {
            return;
        };
        let Some(caret) = self.tracker.caret(view.cursor_row, view.cursor_col) else {
            return;
        };
        if self.hidden_away || (!self.shown && self.reads < 2) {
            return;
        }
        // L'échelle de l'écran du terminal, où la fenêtre va s'afficher, et
        // non celle de l'écran où elle se trouve encore.
        let (screen, scale) = popup.surface.work_area(&popup.window, caret.rect);
        let (x, y, width, height) = place(
            caret.rect,
            caret.cell_width,
            view.word_width,
            measure,
            scale,
            screen,
        );
        popup.surface.show(&popup.window, x, y, width, height);
        self.shown = true;
    }

    /// Nouvelle position du curseur.
    fn read(&mut self, caret: Caret, generation: u64, popup: &Popup) {
        let Some(view) = &self.view else { return };
        // Le focus est passé dans un autre terminal sans que celui-ci ait
        // reçu de frappe : la liste n'a plus rien à faire là.
        let elsewhere = generation != self.show_generation
            && self
                .tracker
                .terminal()
                .is_some_and(|terminal| !terminal.same_terminal(&caret));
        if elsewhere {
            if !self.hidden_away {
                self.hidden_away = true;
                self.hide(popup);
            }
            return;
        }
        let changed = self.tracker.read(caret, view.cursor_row, view.cursor_col);
        self.reads += 1;
        if changed || self.hidden_away || !self.shown {
            self.hidden_away = false;
            self.place(popup);
        }
    }

    fn hide(&mut self, popup: &Popup) {
        self.shown = false;
        popup.surface.hide(&popup.window);
    }

    /// Cache la fenêtre quand on quitte le terminal ; sinon relit la position
    /// du curseur, pour suivre le terminal s'il bouge et corriger une lecture
    /// trop ancienne.
    fn watch(&mut self, popup: &Popup, locate: &mpsc::Sender<(u64, Hint)>) {
        self.next_check = Instant::now() + WATCH;
        let Some(view) = &self.view else { return };
        if let Some(terminal) = self.tracker.terminal() {
            if platform::foreground() != terminal.window {
                if !self.hidden_away {
                    self.hidden_away = true;
                    self.hide(popup);
                }
                return;
            }
        }
        let hint = Hint::from(view);
        self.generation += 1;
        let _ = locate.send((self.generation, hint));
    }
}

/// Message pour `easytab-term`.
fn send(event: Event) {
    if let Ok(line) = serde_json::to_string(&event) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{line}").and_then(|_| stdout.flush());
    }
}

/// Efface le dossier `easytab-overlay.exe.WebView2` qu'une ancienne version a
/// pu laisser à côté du programme installé. Les données de la page vivent
/// dans le dossier local de l'utilisateur (`EasyTab/webview`).
fn remove_stray_webview_data() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // Seulement dans le dossier d'installation, pas dans celui de compilation.
    let installed = exe
        .parent()
        .is_some_and(|dir| dir.ends_with(std::path::Path::new(".easytab").join("bin")));
    if !installed {
        return;
    }
    let mut stray = exe.into_os_string();
    stray.push(".WebView2");
    let _ = std::fs::remove_dir_all(stray);
}
