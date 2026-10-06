// Icône des exécutables Windows, partagée par le `build.rs` de chaque crate
// (`include!`). Sans effet hors de Windows : les builds Linux et macOS n'ont
// besoin d'aucun compilateur de ressources.

/// Compile `assets/easytab.ico` en ressource et la lie aux binaires du crate.
fn icone_windows() {
    // crates/<crate> -> racine du dépôt. Pas de `canonicalize` : sous Windows
    // il donne un chemin `\\?\C:\...` que rc.exe ne comprend pas.
    let racine = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2);
    let ico = racine.expect("racine du dépôt").join("assets").join("easytab.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    assert!(ico.is_file(), "{} introuvable", ico.display());
    // Chemin absolu : rc.exe et windres ne cherchent pas tous les fichiers
    // relativement au .rc.
    let chemin = ico.display().to_string().replace('\\', "\\\\");
    let sortie = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let rc = sortie.join("easytab.rc");
    std::fs::write(&rc, format!("1 ICON \"{chemin}\"\n")).expect("écriture du .rc");
    if let Err(erreur) = embed_resource::compile(&rc, embed_resource::NONE).manifest_required() {
        // Avec MSVC (la CI, les versions publiées), rc.exe est toujours là :
        // une erreur doit se voir. Pour windows-gnu (clippy depuis Linux),
        // windres peut manquer : on construit alors sans icône, avec un avertissement.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            panic!("icône Windows : {erreur}");
        }
        println!("cargo:warning=icône Windows non intégrée : {erreur}");
    }
}
