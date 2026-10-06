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
//! - Sans spec, les options d'une commande sont lues dans son `--help`
//!   ([`HelpSpecs`]) ; les specs de `~/.easytab/specs` passent avant celles
//!   embarquées ([`spec::custom`]).

mod complete;
pub mod config;
mod exec;
mod files;
mod generators;
pub mod help;
pub mod history;
pub mod lang;
pub mod line;
mod osc;
pub mod overlay;
mod project;
pub mod pwsh;
mod rank;
mod session;
pub mod spec;

pub use complete::{Completer, Completion, Kind, Suggestion};
pub use config::Config;
pub use generators::Generators;
pub use help::HelpSpecs;
pub use history::History;
pub use osc::{Marker, OscScanner};
pub use pwsh::PowerShell;
pub use rank::Usage;
pub use session::{Phase, Session};
