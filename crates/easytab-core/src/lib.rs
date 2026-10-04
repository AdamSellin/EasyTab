//! Cœur d'EasyTab : suit l'état du shell à partir du flux brut du terminal.
//!
//! Le wrapper PTY donne à [`Session`] tout ce que le shell affiche et tout ce que
//! l'utilisateur tape. Les scripts d'intégration du shell émettent les marqueurs
//! de prompt `OSC 133` (A : début du prompt, B : début de la saisie, C : début de
//! l'exécution, D : fin de la commande). Avec une copie de l'écran, on en déduit
//! la ligne de commande en cours de saisie.

mod osc;
mod session;

pub use osc::{Marker, OscScanner};
pub use session::{Phase, Session};
