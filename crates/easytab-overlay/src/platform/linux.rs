//! Linux, sous X11 seulement : Wayland interdit de lire la position des
//! autres fenêtres et de placer la sienne. Le terminal est la fenêtre active
//! (`_NET_ACTIVE_WINDOW`) ; on ne connaît pas son curseur, mais on le déduit
//! du cadre de la fenêtre et de la taille de la grille (`estimate`).

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Duration;

use gtk::gdk::prelude::MonitorExt;
use gtk::prelude::WidgetExt;
use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event_loop::{EventLoop, EventLoopBuilder};
use tao::platform::unix::{MonitorHandleExtUnix, WindowBuilderExtUnix, WindowExtUnix};
use tao::window::{Window, WindowBuilder};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtUnix};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use crate::estimate::{caret_from_frame, Hint};
use crate::placement::{Caret, Rect};

/// Intervalle des réveils de la boucle d'événements.
const WAKE: Duration = Duration::from_millis(25);

pub fn event_loop<T: 'static>() -> anyhow::Result<EventLoop<T>> {
    if std::env::var_os("DISPLAY").is_none() {
        anyhow::bail!("pas de serveur X (DISPLAY) : Wayland seul n'est pas pris en charge");
    }
    // GTK passerait par Wayland s'il le pouvait, où la fenêtre ne peut pas
    // se placer elle-même : on reste sous X11 (XWayland au besoin).
    std::env::set_var("GDK_BACKEND", "x11");
    let event_loop = EventLoopBuilder::<T>::with_user_event().build();
    // La boucle GTK de tao ne se réveille pas d'elle-même à l'heure demandée
    // (`ControlFlow::WaitUntil`) : sans ce réveil, la fenêtre ne suivrait
    // plus le terminal entre deux frappes.
    gtk::glib::timeout_add_local(WAKE, || gtk::glib::ControlFlow::Continue);
    Ok(event_loop)
}

pub fn configure(builder: WindowBuilder) -> WindowBuilder {
    builder
        .with_focusable(false)
        .with_skip_taskbar(true)
        // GTK ignore les changements de taille d'une fenêtre fixe ; sans
        // bordure, l'utilisateur ne peut de toute façon pas la redimensionner.
        .with_resizable(true)
}

/// Sous Linux, wry dessine dans un conteneur GTK (WebKitGTK), pas dans la
/// fenêtre elle-même.
pub fn webview<'a>(builder: WebViewBuilder<'a>, window: &'a Window) -> wry::Result<WebView> {
    match window.default_vbox() {
        Some(vbox) => builder.build_gtk(vbox),
        None => builder.build_gtk(window.gtk_window()),
    }
}

pub struct Surface {
    visible: Cell<bool>,
}

impl Surface {
    pub fn new(window: &Window) -> Self {
        // La fenêtre GDK n'existe qu'une fois la fenêtre GTK réalisée : tao
        // en a besoin pour laisser passer les clics.
        window.gtk_window().realize();
        let _ = window.set_ignore_cursor_events(true);
        Self {
            visible: Cell::new(false),
        }
    }

    pub fn show(&self, window: &Window, x: i32, y: i32, width: i32, height: i32) {
        window.set_inner_size(PhysicalSize::new(width.max(1) as u32, height.max(1) as u32));
        window.set_outer_position(PhysicalPosition::new(x, y));
        if !self.visible.replace(true) {
            window.set_visible(true);
        }
    }

    pub fn hide(&self, window: &Window) {
        if self.visible.replace(false) {
            window.set_visible(false);
        }
    }

    /// Zone utilisable (sans les panneaux) de l'écran qui contient le
    /// curseur, en pixels X11, et son échelle.
    pub fn work_area(&self, window: &Window, caret: Rect) -> (Rect, f64) {
        for monitor in window.available_monitors() {
            let scale = monitor.scale_factor();
            let area = monitor.gdk_monitor().workarea();
            let rect = Rect {
                left: (area.x() as f64 * scale).round() as i32,
                top: (area.y() as f64 * scale).round() as i32,
                right: ((area.x() + area.width()) as f64 * scale).round() as i32,
                bottom: ((area.y() + area.height()) as f64 * scale).round() as i32,
            };
            if rect.contains(caret.left, caret.top) {
                return (rect, scale);
            }
        }
        (Rect::EVERYWHERE, window.scale_factor())
    }
}

pub struct Locator;

impl Locator {
    pub fn new() -> Self {
        Self
    }

    pub fn locate(&self, hint: Hint) -> Option<Caret> {
        let display = display()?;
        let window = display.active_window()?;
        let frame = display.frame(window)?;
        // Le cadre lu est celui du contenu de la fenêtre : la barre de titre
        // du gestionnaire de fenêtres n'en fait pas partie.
        let (rect, cell_width) = caret_from_frame(frame, 0, hint)?;
        let caret = Caret {
            rect,
            cell_width,
            window: window as isize,
            element: 0,
        };
        super::log("fenêtre", &caret);
        Some(caret)
    }
}

/// Fenêtre active.
pub fn foreground() -> isize {
    display()
        .and_then(|display| display.active_window())
        .map_or(0, |window| window as isize)
}

/// Connexion au serveur X, partagée par les fils (`None` sans serveur X).
struct Display {
    connection: RustConnection,
    root: u32,
    active_window: u32,
    gtk_frame_extents: u32,
}

fn display() -> Option<&'static Display> {
    static DISPLAY: OnceLock<Option<Display>> = OnceLock::new();
    DISPLAY.get_or_init(Display::connect).as_ref()
}

impl Display {
    fn connect() -> Option<Self> {
        let (connection, screen) = x11rb::connect(None).ok()?;
        let root = connection.setup().roots.get(screen)?.root;
        let atom = |name: &[u8]| {
            connection
                .intern_atom(false, name)
                .ok()?
                .reply()
                .ok()
                .map(|reply| reply.atom)
        };
        let active_window = atom(b"_NET_ACTIVE_WINDOW")?;
        let gtk_frame_extents = atom(b"_GTK_FRAME_EXTENTS")?;
        Some(Self {
            connection,
            root,
            active_window,
            gtk_frame_extents,
        })
    }

    fn active_window(&self) -> Option<u32> {
        let reply = self
            .connection
            .get_property(false, self.root, self.active_window, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let window = reply.value32()?.next();
        window.filter(|&window| window != 0)
    }

    /// Cadre de la fenêtre à l'écran, sans l'ombre que dessinent les
    /// fenêtres GTK autour d'elles (`_GTK_FRAME_EXTENTS`).
    fn frame(&self, window: u32) -> Option<Rect> {
        let geometry = self.connection.get_geometry(window).ok()?.reply().ok()?;
        let origin = self
            .connection
            .translate_coordinates(window, self.root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        let frame = Rect {
            left: origin.dst_x as i32,
            top: origin.dst_y as i32,
            right: origin.dst_x as i32 + geometry.width as i32,
            bottom: origin.dst_y as i32 + geometry.height as i32,
        };
        let extents: Vec<u32> = self
            .connection
            .get_property(
                false,
                window,
                self.gtk_frame_extents,
                AtomEnum::CARDINAL,
                0,
                4,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().map(Iterator::collect))
            .unwrap_or_default();
        Some(match extents[..] {
            [left, right, top, bottom] => Rect {
                left: frame.left + left as i32,
                top: frame.top + top as i32,
                right: frame.right - right as i32,
                bottom: frame.bottom - bottom as i32,
            },
            _ => frame,
        })
    }
}
