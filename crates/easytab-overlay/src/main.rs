//! `easytab-overlay` : fenêtre flottante qui affiche la liste de suggestions
//! sous le curseur du terminal, comme Fig. Lancée par `easytab-term`, elle lit
//! sur son entrée les listes à afficher (une ligne JSON par message, voir
//! `protocol`) et ne prend jamais le focus : le clavier reste au
//! terminal.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod caret;
#[cfg_attr(not(windows), allow(dead_code))]
mod placement;
// Même fichier que `easytab_core::overlay`, sans dépendre du moteur de
// suggestions (et de son moteur JavaScript).
#[cfg_attr(not(windows), allow(dead_code))]
#[path = "../../easytab-core/src/overlay.rs"]
mod protocol;

#[cfg(windows)]
fn main() {
    if let Err(error) = app::run() {
        eprintln!("easytab-overlay : {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    // Le terminal garde la liste dessinée en caractères sur ces systèmes.
    eprintln!("easytab-overlay : pas encore disponible sur ce système");
    std::process::exit(1);
}
