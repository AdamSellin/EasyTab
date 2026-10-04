//! Boucle de la fenêtre (Windows) : une fenêtre sans bordure, transparente,
//! toujours devant, qui ne prend jamais le focus, avec une WebView qui dessine
//! la liste (`popup.html`).

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::protocol::{Event, Request, View};
use tao::dpi::LogicalSize;
use tao::event::{Event as WindowEvent, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::WindowBuilder;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};
use wry::{WebContext, WebViewBuilder};

use crate::caret::{self, Locator};
use crate::placement::{place, Caret, Measure, Rect, Tracker};

/// Fréquence à laquelle on relit la position du curseur et vérifie que le
/// terminal est toujours au premier plan.
const WATCH: Duration = Duration::from_millis(150);
/// Délai de la relecture qui suit chaque frappe.
const SETTLE: Duration = Duration::from_millis(50);

enum UserEvent {
    Request(Request),
    /// `easytab-term` s'est arrêté.
    Closed,
    /// Message de la page : `"ready"` ou ses mesures.
    Page(String),
    /// Résultat d'une recherche du curseur.
    Caret(u64, Option<Caret>),
}

pub fn run() -> anyhow::Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

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

    // La recherche du curseur passe par UI Automation, qui interroge d'autres
    // programmes : elle a son propre fil pour ne jamais bloquer la fenêtre.
    let (locate, locate_rx) = mpsc::channel::<u64>();
    let proxy = event_loop.create_proxy();
    thread::spawn(move || {
        let locator = Locator::new();
        while let Ok(mut generation) = locate_rx.recv() {
            while let Ok(newer) = locate_rx.try_recv() {
                generation = newer;
            }
            if proxy
                .send_event(UserEvent::Caret(generation, locator.locate()))
                .is_err()
            {
                return;
            }
        }
    });

    let window = WindowBuilder::new()
        .with_title("EasyTab")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top(true)
        .with_visible(false)
        .with_focused(false)
        .with_resizable(false)
        .with_skip_taskbar(true)
        .with_undecorated_shadow(false)
        .with_inner_size(LogicalSize::new(320.0, 240.0))
        .build(&event_loop)?;
    let hwnd = window.hwnd();
    // SAFETY: modifie le style de notre propre fenêtre.
    unsafe {
        let style = GetWindowLongPtrW(HWND(hwnd as _), GWL_EXSTYLE);
        SetWindowLongPtrW(
            HWND(hwnd as _),
            GWL_EXSTYLE,
            style | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0) as isize,
        );
    }
    let _ = window.set_ignore_cursor_events(true);

    let mut context =
        WebContext::new(dirs::data_local_dir().map(|dir| dir.join("EasyTab").join("webview")));
    let proxy = event_loop.create_proxy();
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_transparent(true)
        .with_focused(false)
        .with_html(include_str!("popup.html"))
        .with_ipc_handler(move |request| {
            let _ = proxy.send_event(UserEvent::Page(request.into_body()));
        })
        .build(&window)?;

    let mut state = State::default();
    event_loop.run(move |event, _, control_flow| {
        let _ = &window;
        match event {
            WindowEvent::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if Instant::now() >= state.next_check {
                    state.watch(hwnd, &locate);
                }
            }
            WindowEvent::UserEvent(UserEvent::Request(Request::Show(view))) => {
                let json = serde_json::to_string(&view).unwrap_or_default();
                let _ = webview.evaluate_script(&format!("render({json})"));
                state.view = Some(view);
                // La taille de la page arrive avec le nouveau contenu.
                state.measure = None;
                state.generation += 1;
                state.show_generation = state.generation;
                if !state.shown {
                    state.reads = 0;
                }
                let _ = locate.send(state.generation);
                // Relecture peu après : le terminal met à jour son curseur
                // avec un temps de retard.
                state.next_check = Instant::now() + SETTLE;
            }
            WindowEvent::UserEvent(UserEvent::Request(Request::Hide)) => {
                state.view = None;
                state.generation += 1;
                state.hide(hwnd);
            }
            WindowEvent::UserEvent(UserEvent::Page(message)) => {
                if message == "\"ready\"" {
                    send(Event::Ready);
                } else if let Ok(measure) = serde_json::from_str::<Measure>(&message) {
                    state.measure = Some(measure);
                    state.place(hwnd);
                }
            }
            WindowEvent::UserEvent(UserEvent::Caret(generation, caret))
                if generation == state.generation && state.view.is_some() =>
            {
                match caret {
                    Some(caret) => {
                        state.failures = 0;
                        state.read(caret, generation, hwnd);
                    }
                    // Échec passager : on garde la position connue.
                    None if state.tracker.terminal().is_some() => {}
                    None if state.failures < 2 => state.failures += 1,
                    None => {
                        // Le terminal dessinera la liste lui-même.
                        state.view = None;
                        state.failures = 0;
                        state.hide(hwnd);
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
    fn place(&mut self, hwnd: isize) {
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
        let (screen, scale) = work_area(caret.rect);
        let (x, y, width, height) = place(
            caret.rect,
            caret.cell_width,
            view.word_width,
            measure,
            scale,
            screen,
        );
        // SAFETY: déplace et montre notre propre fenêtre, sans l'activer.
        unsafe {
            let _ = SetWindowPos(
                HWND(hwnd as _),
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        self.shown = true;
    }

    /// Nouvelle position du curseur.
    fn read(&mut self, caret: Caret, generation: u64, hwnd: isize) {
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
                self.hide(hwnd);
            }
            return;
        }
        let changed = self.tracker.read(caret, view.cursor_row, view.cursor_col);
        self.reads += 1;
        if changed || self.hidden_away || !self.shown {
            self.hidden_away = false;
            self.place(hwnd);
        }
    }

    fn hide(&mut self, hwnd: isize) {
        self.shown = false;
        hide(hwnd);
    }

    /// Cache la fenêtre quand on quitte le terminal ; sinon relit la position
    /// du curseur, pour suivre le terminal s'il bouge et corriger une lecture
    /// trop ancienne.
    fn watch(&mut self, hwnd: isize, locate: &mpsc::Sender<u64>) {
        self.next_check = Instant::now() + WATCH;
        if self.view.is_none() {
            return;
        }
        if let Some(terminal) = self.tracker.terminal() {
            if caret::foreground() != terminal.window {
                if !self.hidden_away {
                    self.hidden_away = true;
                    self.hide(hwnd);
                }
                return;
            }
        }
        self.generation += 1;
        let _ = locate.send(self.generation);
    }
}

fn hide(hwnd: isize) {
    // SAFETY: cache notre propre fenêtre.
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_HIDE);
    }
}

/// Zone utilisable de l'écran qui contient le curseur (sans la barre des tâches).
fn work_area(caret: Rect) -> (Rect, f64) {
    // SAFETY: lecture des informations d'écran ; `info` vit pendant l'appel.
    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: caret.left,
                y: caret.top,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let (mut dpi_x, mut dpi_y) = (0, 0);
        let scale = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
            Ok(()) if dpi_x > 0 => dpi_x as f64 / 96.0,
            _ => 1.0,
        };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let work = info.rcWork;
            let rect = Rect {
                left: work.left,
                top: work.top,
                right: work.right,
                bottom: work.bottom,
            };
            return (rect, scale);
        }
        let everywhere = Rect {
            left: i32::MIN / 2,
            top: i32::MIN / 2,
            right: i32::MAX / 2,
            bottom: i32::MAX / 2,
        };
        (everywhere, scale)
    }
}

/// Message pour `easytab-term`.
fn send(event: Event) {
    if let Ok(line) = serde_json::to_string(&event) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{line}").and_then(|_| stdout.flush());
    }
}
