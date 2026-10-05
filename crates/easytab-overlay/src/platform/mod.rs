//! Ce qui dépend du système. Chaque module fournit :
//! - `event_loop` : la boucle d'événements, réglée pour une fenêtre qui ne
//!   prend jamais le focus ;
//! - `configure` : les réglages propres au système de la fenêtre ;
//! - `webview` : la page dans la fenêtre ;
//! - `Surface` : rendre la fenêtre transparente aux clics, la montrer, la
//!   cacher et la placer, et donner la zone utilisable
//!   de l'écran du curseur avec son échelle (pixels de l'écran par pixel CSS) ;
//! - `Locator` : la position du curseur du terminal à l'écran ;
//! - `foreground` : la fenêtre au premier plan, comparée à `Caret::window`.
//!
//! Les coordonnées (`Rect`, position de la fenêtre) sont celles de l'écran du
//! système : pixels physiques sous Windows et X11, points sous macOS.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod win;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
pub use win::*;

/// Journal des lectures (`EASYTAB_OVERLAY_LOG=fichier`), pour comprendre un
/// mauvais placement.
pub fn log(source: &str, caret: &crate::placement::Caret) {
    use std::io::Write;
    use std::sync::OnceLock;
    static FILE: OnceLock<Option<std::sync::Mutex<std::fs::File>>> = OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("EASYTAB_OVERLAY_LOG")?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        Some(std::sync::Mutex::new(file))
    });
    if let Some(file) = file {
        if let Ok(mut file) = file.lock() {
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let _ = writeln!(file, "{time} {source} {caret:?}");
        }
    }
}
