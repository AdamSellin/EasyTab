//! `easytab-overlay` : fenêtre flottante qui affiche la liste de suggestions
//! sous le curseur du terminal, comme Fig. Lancée par `easytab-term`, elle lit
//! sur son entrée les listes à afficher (une ligne JSON par message, voir
//! `protocol`) et ne prend jamais le focus : le clavier reste au
//! terminal.
//!
//! La fenêtre, la page et le suivi du curseur sont communs (`app`,
//! `placement`) ; seuls la recherche du curseur, l'écran et le terminal au
//! premier plan dépendent du système (`platform`).

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod app;
// Utilisé par macOS et Linux ; compilé partout pour ses tests.
#[cfg_attr(windows, allow(dead_code))]
mod estimate;
#[cfg_attr(
    not(any(windows, target_os = "macos", target_os = "linux")),
    allow(dead_code)
)]
mod placement;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod platform;
// Même fichier que `easytab_core::overlay`, sans dépendre du moteur de
// suggestions (et de son moteur JavaScript).
#[cfg_attr(
    not(any(windows, target_os = "macos", target_os = "linux")),
    allow(dead_code)
)]
#[path = "../../easytab-core/src/overlay.rs"]
mod protocol;

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn main() {
    if let Err(error) = app::run() {
        eprintln!("easytab-overlay : {error}");
        std::process::exit(1);
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn main() {
    // Le terminal garde la liste dessinée en caractères sur ces systèmes.
    eprintln!("easytab-overlay : pas disponible sur ce système");
    std::process::exit(1);
}
