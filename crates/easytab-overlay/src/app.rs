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
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};
use wry::{WebContext, WebViewBuilder};

use crate::caret::{self, Caret, Locator};
use crate::placement::{place, Measure, Rect};

/// Fréquence à laquelle on vérifie que le terminal est toujours au premier
/// plan et n'a pas bougé.
const WATCH: Duration = Duration::from_millis(150);

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
                state.watch(hwnd, &locate);
            }
            WindowEvent::UserEvent(UserEvent::Request(Request::Show(view))) => {
                let json = serde_json::to_string(&view).unwrap_or_default();
                let _ = webview.evaluate_script(&format!("render({json})"));
                state.view = Some(view);
                // Position et taille sont recalculées pour chaque liste : la
                // fenêtre ne s'affiche qu'avec les deux à jour.
                state.caret = None;
                state.measure = None;
                state.generation += 1;
                let _ = locate.send(state.generation);
            }
            WindowEvent::UserEvent(UserEvent::Request(Request::Hide)) => {
                state.view = None;
                state.generation += 1;
                hide(hwnd);
            }
            WindowEvent::UserEvent(UserEvent::Page(message)) => {
                if message == "\"ready\"" {
                    send(Event::Ready);
                } else if let Ok(measure) = serde_json::from_str::<Measure>(&message) {
                    state.measure = Some(measure);
                    state.place(hwnd, window.scale_factor());
                }
            }
            WindowEvent::UserEvent(UserEvent::Caret(generation, caret)) => {
                if generation != state.generation || state.view.is_none() {
                    return;
                }
                match caret {
                    Some(caret) => {
                        state.caret = Some(caret);
                        state.hidden_away = false;
                        state.place(hwnd, window.scale_factor());
                    }
                    None => {
                        // Le terminal dessinera la liste lui-même.
                        state.view = None;
                        state.caret = None;
                        hide(hwnd);
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
            ControlFlow::WaitUntil(Instant::now() + WATCH)
        } else {
            ControlFlow::Wait
        };
    });
}

#[derive(Default)]
struct State {
    /// Liste demandée par le terminal (`None` : cachée).
    view: Option<View>,
    /// Numéro de la dernière demande : les réponses plus anciennes sont ignorées.
    generation: u64,
    caret: Option<Caret>,
    measure: Option<Measure>,
    /// Le terminal n'est plus au premier plan : la fenêtre est cachée en
    /// attendant qu'il revienne.
    hidden_away: bool,
}

impl State {
    /// Affiche la fenêtre quand le curseur et la taille de la page sont connus.
    fn place(&self, hwnd: isize, scale: f64) {
        let (Some(view), Some(caret), Some(measure)) = (&self.view, self.caret, self.measure)
        else {
            return;
        };
        if self.hidden_away {
            return;
        }
        let screen = work_area(caret.rect);
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
    }

    /// Cache la fenêtre quand on quitte le terminal, la replace quand il
    /// revient ou bouge.
    fn watch(&mut self, hwnd: isize, locate: &mpsc::Sender<u64>) {
        let (Some(_), Some(caret)) = (&self.view, self.caret) else {
            return;
        };
        let foreground = caret::foreground();
        if foreground != caret.window {
            if !self.hidden_away {
                self.hidden_away = true;
                hide(hwnd);
            }
            return;
        }
        if self.hidden_away || caret::window_rect(caret.window) != Some(caret.window_rect) {
            self.hidden_away = false;
            self.generation += 1;
            let _ = locate.send(self.generation);
        }
    }
}

fn hide(hwnd: isize) {
    // SAFETY: cache notre propre fenêtre.
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_HIDE);
    }
}

/// Zone utilisable de l'écran qui contient le curseur (sans la barre des tâches).
fn work_area(caret: Rect) -> Rect {
    // SAFETY: lecture des informations d'écran ; `info` vit pendant l'appel.
    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: caret.left,
                y: caret.top,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let work = info.rcWork;
            return Rect {
                left: work.left,
                top: work.top,
                right: work.right,
                bottom: work.bottom,
            };
        }
    }
    Rect {
        left: i32::MIN / 2,
        top: i32::MIN / 2,
        right: i32::MAX / 2,
        bottom: i32::MAX / 2,
    }
}

/// Message pour `easytab-term`.
fn send(event: Event) {
    if let Ok(line) = serde_json::to_string(&event) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{line}").and_then(|_| stdout.flush());
    }
}
