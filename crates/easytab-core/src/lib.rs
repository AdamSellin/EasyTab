//! Cœur d'EasyTab.
//!
//! - [`Session`] suit l'état du shell à partir du flux brut du terminal. Les
//!   scripts d'intégration émettent les marqueurs de prompt `OSC 133` (A : début
//!   du prompt, B : début de la saisie, C : exécution, D : fin) et le dossier
//!   courant (`OSC 7`). Avec une copie de l'écran, on en déduit la ligne en cours.
//! - [`Completer`] transforme cette ligne en suggestions à partir des specs de
//!   complétion (format Fig).
//! - [`Generators`] calcule en arrière-plan les suggestions dynamiques (branches
//!   git, scripts npm…) en exécutant le JavaScript des specs.

mod complete;
mod exec;
mod files;
mod generators;
pub mod line;
mod osc;
mod session;
pub mod spec;

pub use complete::{Completer, Completion, Kind, Suggestion};
pub use generators::Generators;
pub use osc::{Marker, OscScanner};
pub use session::{Phase, Session};
