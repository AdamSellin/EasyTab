//! `easytab update` : télécharge la dernière version publiée sur GitHub et la
//! réinstalle. Comme les scripts d'installation, il passe par `curl` et `tar`
//! (fournis par Windows 10+, macOS et Linux), et par l'API GitHub avec un
//! jeton quand le dépôt est privé.
//! The archive is checked against the release's `SHA256SUMS` before it is
//! extracted.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

// La fonction `tr` (importée à la racine) et la macro `tr!`.
use crate::tr;

const REPO: &str = "AdamSellin/EasyTab";
/// Release asset listing the SHA-256 of the other assets (`sha256sum` format).
const CHECKSUMS: &str = "SHA256SUMS";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    /// Adresse de l'API : marche aussi pour un dépôt privé, avec un jeton.
    url: String,
    browser_download_url: String,
}

/// Plateforme de ce programme, comme dans le nom des archives publiées.
fn target() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        (os, arch) => bail!(tr!(
            "no published version for {os} {arch}",
            "pas de version publiée pour {os} {arch}"
        )),
    })
}

fn archive_name(target: &str) -> String {
    let extension = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("easytab-{target}.{extension}")
}

/// Vrai si `tag` (`v0.1.2`) n'est pas plus récent que `current` (`0.1.2`).
fn is_current(tag: &str, current: &str) -> bool {
    let numbers = |version: &str| -> Vec<u64> {
        version
            .trim_start_matches('v')
            .split(['.', '-'])
            .map_while(|part| part.parse().ok())
            .collect()
    };
    numbers(tag) <= numbers(current)
}

/// Sous Windows, ceux du système : le `tar` de Git Bash ne lit pas les zip.
fn system_tool(name: &str) -> PathBuf {
    if cfg!(windows) {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let path = Path::new(&root)
                .join("System32")
                .join(format!("{name}.exe"));
            if path.is_file() {
                return path;
            }
        }
    }
    PathBuf::from(name)
}

/// Jeton GitHub : `$GITHUB_TOKEN`, sinon celui que git utilise déjà.
fn github_token() -> Option<String> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            return Some(token);
        }
    }
    let mut child = Command::new("git")
        .args(["credential", "fill"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        use std::io::Write;
        let mut stdin = child.stdin.take()?;
        let _ = stdin.write_all(b"protocol=https\nhost=github.com\n\n");
    }
    let output = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("password="))
        .map(str::to_string)
        .filter(|token| !token.is_empty())
}

/// `curl` silencieux qui échoue sur une erreur HTTP. `curl` ne renvoie pas
/// l'en-tête `Authorization` quand GitHub redirige vers un autre domaine.
fn curl(token: Option<&str>, accept: &str, url: &str, output: Option<&Path>) -> Result<Vec<u8>> {
    let mut command = Command::new(system_tool("curl"));
    command.args(["-fsSL", "-H", &format!("Accept: {accept}")]);
    if let Some(token) = token {
        command.args(["-H", &format!("Authorization: Bearer {token}")]);
    }
    if let Some(output) = output {
        command.arg("-o").arg(output);
    }
    let result = command
        .arg(url)
        .stderr(Stdio::null())
        .output()
        .context(tr("curl not found", "curl introuvable"))?;
    if !result.status.success() {
        bail!(tr!(
            "download of {url} failed",
            "échec du téléchargement de {url}"
        ));
    }
    Ok(result.stdout)
}

/// Expected hash of `name` in a `sha256sum` listing (`<hex>  <name>`, or
/// `<hex> *<name>` in binary mode).
fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim_end().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Fails unless `file` has the hash `SHA256SUMS` gives for `name`.
fn verify(file: &Path, name: &str, sums: &str) -> Result<()> {
    let expected = expected_hash(sums, name).with_context(|| {
        tr!(
            "{name} missing from {CHECKSUMS}",
            "{name} absent de {CHECKSUMS}"
        )
    })?;
    let data = std::fs::read(file).with_context(|| {
        let file = file.display();
        tr!("reading {file}", "lecture de {file}")
    })?;
    if sha256_hex(&data) != expected {
        bail!(tr!(
            "{name} does not match its SHA-256 in {CHECKSUMS}: download corrupted or tampered with, nothing was installed",
            "{name} ne correspond pas à son SHA-256 dans {CHECKSUMS} : téléchargement corrompu ou modifié, rien n'a été installé"
        ));
    }
    Ok(())
}

/// Dernière version publiée, et le jeton qui a permis de la lire (dépôt privé).
fn latest_release() -> Result<(Release, Option<String>)> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let accept = "application/vnd.github+json";
    let (body, token) = match curl(None, accept, &url, None) {
        Ok(body) => (body, None),
        Err(_) => {
            let token = github_token().context(private_repo())?;
            let body = curl(Some(&token), accept, &url, None).context(tr(
                "cannot read the latest version on GitHub",
                "impossible de lire la dernière version sur GitHub",
            ))?;
            (body, Some(token))
        }
    };
    let release = serde_json::from_slice(&body).context(tr(
        "unreadable answer from GitHub",
        "réponse de GitHub illisible",
    ))?;
    Ok((release, token))
}

/// Erreur quand le dépôt est privé et qu'aucun jeton ne marche.
fn private_repo() -> &'static str {
    tr(
        "private repository: connect git to GitHub, or set GITHUB_TOKEN, then try again",
        "dépôt privé : connecte git à GitHub, ou définis GITHUB_TOKEN, puis relance",
    )
}

pub fn run(force: bool, installed_shells: &[&str]) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let target = target()?;
    let (release, token) = latest_release()?;
    let tag = &release.tag_name;
    if is_current(tag, current) && !force {
        println!(
            "{}",
            tr!(
                "EasyTab is up to date ({tag}).",
                "EasyTab est à jour ({tag})."
            )
        );
        return Ok(());
    }
    let name = archive_name(target);
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .with_context(|| {
            tr!(
                "{name} missing from version {tag}",
                "{name} absent de la version {tag}"
            )
        })?;
    let checksums = release
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUMS)
        .with_context(|| {
            tr!(
                "{CHECKSUMS} missing from version {tag}: cannot check the download",
                "{CHECKSUMS} absent de la version {tag} : impossible de vérifier le téléchargement"
            )
        })?;

    let dir = std::env::temp_dir().join(format!("easytab-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| {
        let dir = dir.display();
        tr!("creating {dir}", "création de {dir}")
    })?;
    let result = download_and_install(
        &dir,
        asset,
        checksums,
        token.as_deref(),
        target,
        installed_shells,
    );
    let _ = std::fs::remove_dir_all(&dir);
    result?;
    println!(
        "{}",
        tr!(
            "EasyTab updated: {current} → {tag}. Reopen your terminals to use it.",
            "EasyTab mis à jour : {current} → {tag}. Rouvre tes terminaux pour l'utiliser."
        )
    );
    Ok(())
}

/// Downloads `asset` into `dir`: direct link first; private repository:
/// through the API, with a token.
fn download(dir: &Path, asset: &Asset, token: Option<&str>) -> Result<PathBuf> {
    let file = dir.join(&asset.name);
    let direct = match token {
        Some(_) => None,
        None => curl(None, "*/*", &asset.browser_download_url, Some(&file)).ok(),
    };
    if direct.is_none() {
        let token = token.map(str::to_string).or_else(github_token);
        curl(
            token.as_deref(),
            "application/octet-stream",
            &asset.url,
            Some(&file),
        )
        .context(private_repo())?;
    }
    Ok(file)
}

fn download_and_install(
    dir: &Path,
    asset: &Asset,
    checksums: &Asset,
    token: Option<&str>,
    target: &str,
    installed_shells: &[&str],
) -> Result<()> {
    let name = &asset.name;
    println!("{}", tr!("Downloading {name}", "Téléchargement de {name}"));
    let archive = download(dir, asset, token)?;
    let sums = std::fs::read_to_string(download(dir, checksums, token)?)
        .with_context(|| tr!("{CHECKSUMS} unreadable", "{CHECKSUMS} illisible"))?;
    verify(&archive, name, &sums)?;
    let status = Command::new(system_tool("tar"))
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(dir)
        .status()
        .context(tr("tar not found", "tar introuvable"))?;
    if !status.success() {
        bail!(tr!(
            "cannot extract {name}",
            "décompression de {name} impossible"
        ));
    }
    let easytab = dir
        .join(format!("easytab-{target}"))
        .join(format!("easytab{}", std::env::consts::EXE_SUFFIX));
    // La nouvelle version s'installe elle-même, pour chaque shell déjà
    // configuré : ses blocs remplacent les anciens.
    let shells: Vec<Option<&str>> = if installed_shells.is_empty() {
        vec![None]
    } else {
        installed_shells.iter().copied().map(Some).collect()
    };
    for shell in shells {
        let mut command = Command::new(&easytab);
        command.arg("install");
        if let Some(shell) = shell {
            command.args(["--shell", shell]);
        }
        let status = command.status().with_context(|| {
            let easytab = easytab.display();
            tr!("running {easytab}", "lancement de {easytab}")
        })?;
        if !status.success() {
            bail!(tr(
                "installing the new version failed",
                "l'installation de la nouvelle version a échoué"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_archives_like_the_release() {
        assert_eq!(
            archive_name("x86_64-pc-windows-msvc"),
            "easytab-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            archive_name("aarch64-apple-darwin"),
            "easytab-aarch64-apple-darwin.tar.gz"
        );
        assert!(target().is_ok());
    }

    #[test]
    fn compares_versions() {
        assert!(is_current("v0.1.2", "0.1.2"));
        assert!(is_current("0.1.2", "0.1.2"));
        assert!(!is_current("v0.1.3", "0.1.2"));
        assert!(!is_current("v0.10.0", "0.9.1"));
        // Jamais de retour en arrière sans --force.
        assert!(is_current("v0.1.1", "0.1.2"));
    }

    #[test]
    fn reads_github_releases() {
        let release: Release = serde_json::from_str(
            r#"{"tag_name": "v0.1.1", "name": "EasyTab v0.1.1", "assets": [
                {"name": "install.sh", "url": "https://api.github.com/repos/a/b/releases/assets/1",
                 "browser_download_url": "https://github.com/a/b/releases/download/v0.1.1/install.sh",
                 "size": 10}]}"#,
        )
        .unwrap();
        assert_eq!(release.tag_name, "v0.1.1");
        assert_eq!(release.assets[0].name, "install.sh");
    }

    #[test]
    fn reads_sha256sums() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!(
            "{a}  easytab-x.zip
{b} *easytab-y.tar.gz
not a line
"
        );
        assert_eq!(expected_hash(&sums, "easytab-x.zip"), Some(a));
        assert_eq!(
            expected_hash(&sums, "easytab-y.tar.gz"),
            Some("b".repeat(64))
        );
        assert_eq!(expected_hash(&sums, "easytab-z.zip"), None);
        // Short hash: ignored.
        assert_eq!(expected_hash("abc  easytab-x.zip", "easytab-x.zip"), None);
    }

    #[test]
    fn checks_the_archive_hash() {
        let dir = std::env::temp_dir().join(format!("easytab-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("easytab-x.zip");
        std::fs::write(&file, b"abc").unwrap();
        // SHA-256 of "abc".
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(b"abc"), good);
        assert!(verify(&file, "easytab-x.zip", &format!("{good}  easytab-x.zip")).is_ok());
        let bad = "0".repeat(64);
        assert!(verify(&file, "easytab-x.zip", &format!("{bad}  easytab-x.zip")).is_err());
        assert!(verify(&file, "easytab-x.zip", "").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
