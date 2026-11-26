//! Self-update: find out whether a newer Opentern has been released, and swap
//! the installed app for it.
//!
//! The feed is GitHub's "latest release" API (or `OPENTERN_UPDATE_FEED`, a URL
//! or local JSON file of the same shape, for testing). Installing downloads
//! the build for this platform, checks it against the release's SHA256SUMS
//! (required),
//! and replaces the program in place:
//!
//! - Linux: the `opentern` binary (renaming over a running binary is safe; the
//!   running process keeps the old file until it exits).
//! - macOS: the whole `Opentern.app` bundle.
//! - Windows: `opentern.exe`, after renaming the running one aside.
//!
//! While the repository is private, GitHub answers anonymous requests with
//! 404, so the check and the download fall back to the GitHub CLI (`gh`) when
//! it's installed and signed in. Opentern never sees or stores the token.
//!
//! Your data folder is never touched. If the app lives somewhere Opentern
//! can't write (a package manager's /usr/bin, an all-users Program Files
//! install), [`Plan::Manual`] says to update the usual way instead.

use crate::{Error, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const REPO: &str = "ahaan-shah/opentern";
const LATEST: &str = "https://api.github.com/repos/ahaan-shah/opentern/releases/latest";
const AGENT: &str = concat!("opentern/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// "0.1.7", without the leading v.
    pub version: String,
    /// The release page, for manual downloads and release notes.
    pub page: String,
    pub notes: String,
    pub assets: Vec<(String, String)>,
    /// The release tag ("v0.1.7").
    pub tag: String,
    /// Found through the GitHub CLI (a private repository), so downloads go
    /// through it too.
    pub gh: bool,
}

impl Release {
    fn asset(&self, name: &str) -> Option<&str> {
        self.assets.iter().find(|(n, _)| n == name).map(|(_, u)| u.as_str())
    }
}

/// "0.1.10" → (0, 1, 10). Pre-release suffixes are ignored.
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u32>().ok());
    Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The `Error::Http` message when GitHub couldn't be reached at all (no
/// connection, DNS failure, timeout), as opposed to answering with an error.
pub const OFFLINE: &str = "offline";

/// How long each part of a request may take. Without them a server that
/// never answers holds a download for as long as the system's own TCP
/// timeout (about two minutes per address on Linux), which looked like the
/// update had frozen. The connect budget is shared out across a host's
/// addresses, so one that doesn't answer falls through to the next quickly.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resolve: Duration,
    pub connect: Duration,
    pub response: Duration,
    /// For one ranged piece of a download, start to finish.
    pub piece: Duration,
    /// How big a piece is: small enough that a stalled one costs little.
    pub piece_bytes: u64,
    /// Tries for a piece that makes no progress at all.
    pub tries: u32,
}

impl Limits {
    pub const NORMAL: Limits = Limits {
        resolve: Duration::from_secs(15),
        connect: Duration::from_secs(10),
        response: Duration::from_secs(30),
        piece: Duration::from_secs(90),
        piece_bytes: 2 << 20,
        tries: 4,
    };
}

fn agent(l: &Limits) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_resolve(Some(l.resolve))
        .timeout_connect(Some(l.connect))
        .timeout_send_request(Some(l.response))
        .timeout_recv_response(Some(l.response))
        .user_agent(AGENT)
        .build()
        .into()
}

fn http_error(e: ureq::Error) -> Error {
    match e {
        ureq::Error::StatusCode(404) => Error::Http("no published release found".into()),
        ureq::Error::Io(_)
        | ureq::Error::Timeout(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::BodyStalled => Error::Http(OFFLINE.into()),
        e => Error::Http(e.to_string()),
    }
}

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    let l = Limits::NORMAL;
    agent(&l)
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .config()
        .timeout_global(Some(l.piece))
        .build()
        .call()
        .map_err(http_error)
}

/// Runs the GitHub CLI with fixed arguments; its stdout on success.
fn gh(args: &[&str]) -> Option<Vec<u8>> {
    let out = std::process::Command::new("gh")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// The feed's JSON, and whether it came through the GitHub CLI.
fn feed_json() -> Result<(String, bool)> {
    let custom = std::env::var("OPENTERN_UPDATE_FEED").ok();
    let feed = custom.clone().unwrap_or_else(|| LATEST.to_string());
    if !feed.starts_with("http://") && !feed.starts_with("https://") {
        return Ok((std::fs::read_to_string(feed.trim_start_matches("file://"))?, false));
    }
    match get(&feed) {
        Ok(mut res) => Ok((
            res.body_mut()
                .read_to_string()
                .map_err(|e| Error::Http(e.to_string()))?,
            false,
        )),
        // A private repository looks missing to anonymous requests.
        Err(Error::Http(m)) if custom.is_none() && m != OFFLINE => {
            match gh(&["api", &format!("repos/{REPO}/releases/latest")]) {
                Some(out) => Ok((String::from_utf8_lossy(&out).into_owned(), true)),
                None => Err(Error::Http(m)),
            }
        }
        Err(e) => Err(e),
    }
}

fn parse_feed(json: &str) -> Result<Release> {
    #[derive(serde::Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    #[derive(serde::Deserialize)]
    struct Latest {
        tag_name: String,
        #[serde(default)]
        html_url: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    let l: Latest = serde_json::from_str(json)?;
    Ok(Release {
        version: l.tag_name.trim_start_matches('v').to_string(),
        page: l.html_url,
        notes: l.body.unwrap_or_default(),
        assets: l.assets.into_iter().map(|a| (a.name, a.browser_download_url)).collect(),
        tag: l.tag_name,
        gh: false,
    })
}

/// The newest release if it's newer than this build, else None.
pub fn check() -> Result<Option<Release>> {
    let (json, via_gh) = feed_json()?;
    let mut r = parse_feed(&json)?;
    r.gh = via_gh;
    Ok(is_newer(&r.version, env!("CARGO_PKG_VERSION")).then_some(r))
}

// ------------------------------------------------------------------ install

/// What installing an update would do on this machine.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    /// Download `asset` and replace `target` (a binary, or a .app bundle).
    Replace { asset: String, target: PathBuf },
    /// Can't update in place; the reason, shown next to a download link.
    Manual(String),
}

fn asset_name() -> Option<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    if cfg!(target_os = "linux") {
        Some(format!("opentern-{arch}-unknown-linux-gnu.tar.gz"))
    } else if cfg!(target_os = "macos") {
        Some("Opentern-macos-universal.zip".into())
    } else if cfg!(windows) {
        Some(format!(
            "Opentern-windows-{}.zip",
            if arch == "x86_64" { "x64" } else { "arm64" }
        ))
    } else {
        None
    }
}

fn writable_dir(dir: &Path) -> bool {
    let probe = dir.join(format!(".opentern-write-test-{}", std::process::id()));
    let ok = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Works out how this copy of Opentern can be updated.
pub fn plan() -> Plan {
    let Some(asset) = asset_name() else {
        return Plan::Manual("Updates aren't available for this platform".into());
    };
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
        return Plan::Manual("Couldn't find where Opentern is installed".into());
    };
    let target = if cfg!(target_os = "macos") {
        // .../Opentern.app/Contents/MacOS/opentern
        match exe.ancestors().nth(3) {
            Some(app) if app.extension().is_some_and(|e| e == "app") => app.to_path_buf(),
            _ => return Plan::Manual("This copy of Opentern isn't an app bundle".into()),
        }
    } else {
        exe
    };
    let in_system = ["/usr/", "/opt/", "/nix/", "/snap/", "/var/lib/flatpak/"]
        .iter()
        .any(|p| target.starts_with(p));
    if cfg!(target_os = "linux") && in_system {
        return Plan::Manual("Opentern was installed by your package manager; update it there".into());
    }
    let dir = target.parent().unwrap_or(Path::new("."));
    if !writable_dir(dir) {
        return Plan::Manual(format!("Opentern can't write to {}", shown(dir)));
    }
    Plan::Replace { asset, target }
}

fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Where an install has got to, for the progress shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// How much of the download is in, 0–1.
    Downloading(f32),
    /// The connection dropped or stalled; trying again from where it got to.
    Retrying,
    /// Checking the download against SHA256SUMS.
    Checking,
    /// Unpacking and swapping it in.
    Installing,
}

#[cfg(test)]
fn download(url: &str, to: &Path, progress: &dyn Fn(f32)) -> Result<()> {
    download_with(url, to, &Limits::NORMAL, progress, &|| {})
}

/// Downloads `url` into `to` a piece at a time (HTTP ranges), each piece
/// with its own time limit. A piece that stalls or drops is asked for again
/// from the byte it got to, up to `tries` times running without progress.
/// A server that ignores ranges sends the whole file in one go.
fn download_with(url: &str, to: &Path, l: &Limits, progress: &dyn Fn(f32), retrying: &dyn Fn()) -> Result<()> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        std::fs::copy(url.trim_start_matches("file://"), to)?;
        progress(1.0);
        return Ok(());
    }
    let agent = agent(l);
    let mut out = std::fs::File::create(to)?;
    let mut done = 0u64;
    let mut total: Option<u64> = None;
    let mut fails = 0u32;
    // The last request failed (dropped, stalled, refused a connection).
    let mut again = false;
    let mut last_err = Error::Http(OFFLINE.into());
    let mut buf = vec![0u8; 1 << 16];
    loop {
        if total.is_some_and(|t| done >= t) {
            break;
        }
        if fails >= l.tries {
            return Err(last_err);
        }
        if again {
            retrying();
            std::thread::sleep(Duration::from_millis(400 * u64::from(fails.max(1))));
        }
        again = false;
        let end = done + l.piece_bytes - 1;
        let res = agent
            .get(url)
            .header("Range", format!("bytes={done}-{end}"))
            .config()
            .timeout_global(Some(l.piece))
            .build()
            .call();
        let mut res = match res {
            Ok(r) => r,
            // Asked past the end: the file was exactly a multiple of pieces.
            Err(ureq::Error::StatusCode(416)) if done > 0 && total.is_none() => break,
            Err(e) => {
                let e = http_error(e);
                // A missing file or a refusal won't get better by asking again.
                if !matches!(&e, Error::Http(m) if m == OFFLINE) {
                    return Err(e);
                }
                last_err = e;
                fails += 1;
                again = true;
                continue;
            }
        };
        let whole = res.status() == 200;
        if whole {
            // No ranges here: start again from the top, in one go.
            if done > 0 {
                out.set_len(0)?;
                use std::io::Seek;
                out.seek(std::io::SeekFrom::Start(0))?;
                done = 0;
            }
            total = res.body().content_length();
        } else if let Some(t) = res
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|v| v.trim().parse::<u64>().ok())
        {
            total = Some(t);
        }
        let before = done;
        let mut reader = res.body_mut().as_reader();
        let read = loop {
            match reader.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => {
                    out.write_all(&buf[..n])?;
                    done += n as u64;
                    if let Some(t) = total.filter(|t| *t > 0) {
                        progress((done as f32 / t as f32).min(1.0));
                    }
                }
                Err(e) => break Err(e),
            }
        };
        match read {
            Ok(()) if whole || total.is_none() => {
                // In one go, or a server that never says how big: done when
                // it ends, if it sent what it promised.
                if total.is_some_and(|t| done < t) {
                    last_err = Error::Http(OFFLINE.into());
                    fails += 1;
                    again = true;
                    continue;
                }
                total = Some(done);
            }
            Ok(()) => {}
            // Stalled past the piece's time limit, or the connection dropped:
            // ask again from where it got to.
            Err(_) => {
                last_err = Error::Http(OFFLINE.into());
                again = true;
            }
        }
        if done > before {
            fails = 0;
        } else {
            fails += 1;
        }
    }
    out.sync_all()?;
    progress(1.0);
    Ok(())
}

fn run(cmd: &mut std::process::Command) -> Result<()> {
    let out = cmd.output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Msg(format!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// Downloads `release` and installs it over this copy. `progress` gets the
/// download fraction (0–1). On success the new version runs on next launch.
pub fn install(release: &Release, step: &dyn Fn(Step)) -> Result<()> {
    let (asset, target) = match plan() {
        Plan::Replace { asset, target } => (asset, target),
        Plan::Manual(why) => return Err(Error::Msg(why)),
    };
    let url = release
        .asset(&asset)
        .ok_or_else(|| Error::Msg(format!("this release has no {asset}")))?;
    // Work next to the target so the final rename never crosses filesystems.
    let parent = target.parent().unwrap_or(Path::new("."));
    let work = parent.join(format!(".opentern-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    // A private repository's files need the GitHub CLI's sign-in.
    let fetch = |name: &str, url: &str, to: &Path, progress: &dyn Fn(f32)| -> Result<()> {
        if !release.gh {
            return download_with(url, to, &Limits::NORMAL, progress, &|| step(Step::Retrying));
        }
        let dir = to.parent().unwrap_or(Path::new(".")).to_string_lossy().into_owned();
        gh(&[
            "release",
            "download",
            &release.tag,
            "-R",
            REPO,
            "-p",
            name,
            "-D",
            &dir,
            "--clobber",
        ])
        .ok_or_else(|| Error::Http(format!("couldn't download {name} with the GitHub CLI")))?;
        progress(1.0);
        Ok(())
    };
    let result = (|| {
        let file = work.join(&asset);
        step(Step::Downloading(0.0));
        fetch(&asset, url, &file, &|p| step(Step::Downloading(p)))?;
        step(Step::Checking);

        // Every release carries SHA256SUMS; never install one that doesn't.
        let sums_url = release
            .asset("SHA256SUMS")
            .ok_or_else(|| Error::Msg("this release has no SHA256SUMS, so it can't be checked".into()))?;
        {
            let sums = work.join("SHA256SUMS");
            fetch("SHA256SUMS", sums_url, &sums, &|_| {})?;
            let text = std::fs::read_to_string(&sums)?;
            let expected = text
                .lines()
                .filter_map(|l| l.split_once(char::is_whitespace))
                .find(|(_, name)| name.trim().trim_start_matches('*') == asset)
                .map(|(h, _)| h.to_ascii_lowercase())
                .ok_or_else(|| Error::Msg(format!("{asset} isn't in SHA256SUMS")))?;
            if sha256_file(&file)? != expected {
                return Err(Error::Msg("the download is corrupted (checksum mismatch)".into()));
            }
        }

        step(Step::Installing);
        let unpacked = work.join("unpacked");
        std::fs::create_dir_all(&unpacked)?;
        if asset.ends_with(".tar.gz") {
            run(std::process::Command::new("tar")
                .arg("-xzf")
                .arg(&file)
                .arg("-C")
                .arg(&unpacked))?;
        } else if cfg!(target_os = "macos") {
            run(std::process::Command::new("ditto")
                .args(["-x", "-k"])
                .arg(&file)
                .arg(&unpacked))?;
        } else {
            unzip(&file, &unpacked)?;
        }
        let fresh = find(&unpacked, target.file_name().unwrap_or_default())
            .ok_or_else(|| Error::Msg("the download doesn't contain Opentern".into()))?;
        swap(&fresh, &target)
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Unpacks a zip in-process. Not `tar`: on Windows a Git or MSYS `tar`
/// earlier on PATH reads "C:\…" as a remote host, and starting a console
/// program from a GUI app flashes a terminal window.
fn unzip(file: &Path, to: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(file)?)
        .map_err(|e| Error::Msg(format!("the download isn't a valid zip ({e})")))?;
    // `extract` refuses entries that would land outside `to`.
    archive
        .extract(to)
        .map_err(|e| Error::Msg(format!("couldn't unpack the download ({e})")))
}

/// Finds `name` anywhere under `dir` (archives nest it in a folder).
fn find(dir: &Path, name: &std::ffi::OsStr) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.file_name() == Some(name) {
            return Some(p);
        }
        if p.is_dir()
            && p.extension().is_none_or(|e| e != "app")
            && let Some(found) = find(&p, name)
        {
            return Some(found);
        }
    }
    None
}

/// Puts `fresh` where `target` is, keeping the old one until the new one is
/// in place, so a failure leaves the working install untouched.
fn swap(fresh: &Path, target: &Path) -> Result<()> {
    let old = target.with_file_name(format!(
        ".{}.old",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    if old.is_dir() {
        let _ = std::fs::remove_dir_all(&old);
    } else {
        let _ = std::fs::remove_file(&old);
    }
    #[cfg(unix)]
    if fresh.is_file() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(fresh, std::fs::Permissions::from_mode(0o755))?;
    }
    rename(target, &old)?;
    if let Err(e) = rename(fresh, target) {
        let _ = rename(&old, target);
        return Err(e.into());
    }
    // A running Windows exe can be renamed but not deleted; it's cleaned up
    // on the next launch by `cleanup`.
    if old.is_dir() {
        let _ = std::fs::remove_dir_all(&old);
    } else {
        let _ = std::fs::remove_file(&old);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr")
            .args(["-dr", "com.apple.quarantine"])
            .arg(target)
            .output();
    }
    Ok(())
}

/// `fs::rename`, retried for up to two seconds on Windows, where an
/// antivirus scan of a freshly written exe holds it open for a moment.
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(_) if cfg!(windows) && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            r => return r,
        }
    }
}

/// A path as people write it: without Windows' `\\?\` prefix, which
/// `canonicalize` adds.
fn shown(p: &Path) -> String {
    let s = p.display().to_string();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => rest.to_string(),
        _ => s,
    }
}

/// Removes what a previous update left behind: the old Windows exe, and
/// the work folder of an update that was closed or crashed part way (each
/// is named after the process that made it; this one's is kept).
pub fn cleanup() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = exe.with_file_name(format!(
        ".{}.old",
        exe.file_name().unwrap_or_default().to_string_lossy()
    ));
    let _ = std::fs::remove_file(old);
    if let Some(dir) = exe.parent() {
        clean_work_dirs(dir, std::process::id());
    }
}

fn clean_work_dirs(dir: &Path, keep: u32) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mine = format!(".opentern-update-{keep}");
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let pid = name.strip_prefix(".opentern-update-");
        if pid.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            && name != mine
            && e.file_type().is_ok_and(|t| t.is_dir())
        {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn versions() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("v0.2.0", "0.1.6"));
        assert!(!is_newer("0.1.6", "0.1.6"));
        assert!(!is_newer("0.1.5", "0.1.6"));
        assert!(!is_newer("garbage", "0.1.6"));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("1.2.3-beta.1"), Some((1, 2, 3)));
    }

    #[test]
    fn reads_github_feed() {
        let r = parse_feed(
            r#"{"tag_name":"v0.1.7","html_url":"https://github.com/x/y/releases/tag/v0.1.7","body":"Notes",
                "assets":[{"name":"SHA256SUMS","browser_download_url":"https://e/SHA256SUMS"}]}"#,
        )
        .unwrap();
        assert_eq!(r.version, "0.1.7");
        assert_eq!(r.asset("SHA256SUMS"), Some("https://e/SHA256SUMS"));
    }

    #[test]
    fn swap_replaces_and_keeps_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("opentern");
        std::fs::write(&target, "old").unwrap();
        let fresh = dir.path().join("new-opentern");
        std::fs::write(&fresh, "new").unwrap();
        swap(&fresh, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(!dir.path().join(".opentern.old").exists());
        assert!(!fresh.exists());
    }

    #[test]
    fn install_from_local_feed() {
        // A fake release on disk: a tarball with a "new" binary and its checksum.
        let dir = tempfile::tempdir().unwrap();
        let stage = dir.path().join("opentern-x86_64-unknown-linux-gnu");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join("opentern"), "new build").unwrap();
        let tar = dir.path().join("pkg.tar.gz");
        run(std::process::Command::new("tar")
            .arg("-czf")
            .arg(&tar)
            .arg("-C")
            .arg(dir.path())
            .arg("opentern-x86_64-unknown-linux-gnu"))
        .unwrap();
        let sum = sha256_file(&tar).unwrap();
        let sums = dir.path().join("SUMS");
        std::fs::write(&sums, format!("{sum}  pkg.tar.gz\n")).unwrap();
        let release = Release {
            version: "9.9.9".into(),
            page: String::new(),
            notes: String::new(),
            assets: vec![
                ("pkg.tar.gz".into(), tar.display().to_string()),
                ("SHA256SUMS".into(), sums.display().to_string()),
            ],
            tag: "v9.9.9".into(),
            gh: false,
        };
        // Exercise the same steps install() runs, against a stand-in target.
        let target = dir.path().join("bin").join("opentern");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "old build").unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(work.join("unpacked")).unwrap();
        let file = work.join("pkg.tar.gz");
        download(release.asset("pkg.tar.gz").unwrap(), &file, &|_| {}).unwrap();
        assert_eq!(sha256_file(&file).unwrap(), sum);
        run(std::process::Command::new("tar")
            .arg("-xzf")
            .arg(&file)
            .arg("-C")
            .arg(work.join("unpacked")))
        .unwrap();
        let fresh = find(&work.join("unpacked"), std::ffi::OsStr::new("opentern")).unwrap();
        swap(&fresh, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new build");
    }
    /// A stand-in download server on localhost. `serve` answers each
    /// request; it gets the request's Range start and how many requests came
    /// before.
    fn server(serve: impl Fn(&mut std::net::TcpStream, u64, usize) + Send + Sync + 'static) -> String {
        use std::io::BufRead;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let serve = std::sync::Arc::new(serve);
        std::thread::spawn(move || {
            for (n, s) in l.incoming().enumerate() {
                let Ok(mut s) = s else { continue };
                let serve = serve.clone();
                std::thread::spawn(move || {
                    let mut r = std::io::BufReader::new(s.try_clone().unwrap());
                    let mut from = 0;
                    loop {
                        let mut line = String::new();
                        if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            from = v.split('-').next().unwrap().trim().parse().unwrap_or(0);
                        }
                    }
                    serve(&mut s, from, n);
                });
            }
        });
        format!("http://127.0.0.1:{port}/file")
    }

    fn body() -> Vec<u8> {
        (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect()
    }

    /// The `n`-th request's answer: a 206 for `from..`, at most `piece` bytes,
    /// cut short after `cut` bytes (then stalls for `stall`, or hangs up).
    fn ranged(s: &mut std::net::TcpStream, from: u64, piece: u64, cut: Option<usize>, stall: Option<Duration>) {
        let all = body();
        let end = (from + piece).min(all.len() as u64);
        let part = &all[from as usize..end as usize];
        let head = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
            part.len(),
            from,
            end - 1,
            all.len()
        );
        let _ = s.write_all(head.as_bytes());
        let send = cut.unwrap_or(part.len()).min(part.len());
        let _ = s.write_all(&part[..send]);
        let _ = s.flush();
        if let Some(d) = stall {
            std::thread::sleep(d);
        }
    }

    fn quick() -> Limits {
        Limits {
            resolve: Duration::from_secs(2),
            connect: Duration::from_millis(400),
            response: Duration::from_secs(2),
            piece: Duration::from_millis(700),
            piece_bytes: 64 << 10,
            tries: 4,
        }
    }

    fn fetch(url: &str, l: &Limits) -> (Result<()>, Vec<u8>, usize, f32) {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("f");
        let retries = std::sync::atomic::AtomicUsize::new(0);
        let last = std::sync::Mutex::new(0.0f32);
        let r = download_with(url, &to, l, &|p| *last.lock().unwrap() = p, &|| {
            retries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let got = std::fs::read(&to).unwrap_or_default();
        let p = *last.lock().unwrap();
        (r, got, retries.into_inner(), p)
    }

    #[test]
    fn downloads_in_pieces() {
        let l = quick();
        let url = server(move |s, from, _| ranged(s, from, 64 << 10, None, None));
        let (r, got, retries, p) = fetch(&url, &l);
        r.unwrap();
        assert_eq!(got, body());
        assert_eq!((retries, p), (0, 1.0));
    }

    /// The frozen update: a piece that stops arriving is asked for again
    /// from the byte it got to, within the piece's time limit.
    #[test]
    fn a_stalled_piece_resumes() {
        let l = quick();
        let url = server(move |s, from, n| {
            let stall = (n == 1).then_some(Duration::from_secs(5));
            ranged(s, from, 64 << 10, stall.map(|_| 1000), stall)
        });
        let t = std::time::Instant::now();
        let (r, got, retries, _) = fetch(&url, &l);
        r.unwrap();
        assert_eq!(got, body(), "resumed at the right byte");
        assert!(retries >= 1);
        assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
    }

    #[test]
    fn a_dropped_connection_resumes() {
        let l = quick();
        let url = server(move |s, from, n| {
            let cut = matches!(n, 0 | 2).then_some(5000);
            ranged(s, from, 64 << 10, cut, None)
        });
        let (r, got, retries, _) = fetch(&url, &l);
        r.unwrap();
        // Each cut piece still got somewhere, so it goes on from there.
        assert_eq!(got, body());
        assert!(retries >= 1);
    }

    #[test]
    fn a_server_without_ranges_sends_it_whole() {
        let l = quick();
        let url = server(move |s, _, _| {
            let all = body();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                all.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&all);
        });
        let (r, got, _, p) = fetch(&url, &l);
        r.unwrap();
        assert_eq!(got, body());
        assert_eq!(p, 1.0);
    }

    /// One that never gets anywhere gives up after its tries, with the
    /// "offline" error the app words for people, instead of hanging.
    #[test]
    fn a_server_that_never_answers_gives_up() {
        let l = quick();
        let url = server(move |_, _, _| std::thread::sleep(Duration::from_secs(10)));
        let t = std::time::Instant::now();
        let (r, _, retries, _) = fetch(&url, &l);
        assert!(matches!(r, Err(Error::Http(m)) if m == OFFLINE));
        assert_eq!(retries, 3);
        assert!(t.elapsed() < Duration::from_secs(9), "{:?}", t.elapsed());
    }

    /// An address that takes no connections at all (a black hole, like the
    /// GitHub server that froze 0.3.4's updater) hits the connect limit, not
    /// the system's two-minute TCP timeout.
    #[test]
    fn an_address_that_never_connects_gives_up_quickly() {
        let l = Limits { tries: 2, ..quick() };
        let t = std::time::Instant::now();
        // TEST-NET-1: reserved, never routed.
        let (r, _, _, _) = fetch("http://192.0.2.1:81/file", &l);
        assert!(r.is_err());
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    }

    #[test]
    fn a_missing_file_isnt_retried() {
        let l = quick();
        let url = server(move |s, _, _| {
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        });
        let (r, _, retries, _) = fetch(&url, &l);
        assert!(r.is_err());
        assert_eq!(retries, 0);
    }

    #[test]
    fn leftover_work_folders_are_cleared() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["123", "456", "789"] {
            let d = dir.path().join(format!(".opentern-update-{n}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("x.tar.gz"), "half").unwrap();
        }
        for keep in [".opentern-update-", ".opentern-update-abc", "opentern"] {
            std::fs::write(dir.path().join(keep), "").unwrap();
        }
        clean_work_dirs(dir.path(), 456);
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                ".opentern-update-",
                ".opentern-update-456",
                ".opentern-update-abc",
                "opentern"
            ]
        );
    }

    /// The real release, through GitHub's redirect: pieces come back as 206
    /// and the result matches SHA256SUMS. Uses the network.
    #[test]
    #[ignore]
    fn live_release_download() {
        let base = std::env::var("RELEASE_BASE")
            .unwrap_or_else(|_| "https://github.com/ahaan-shah/opentern/releases/latest/download".into());
        let dir = tempfile::tempdir().unwrap();
        let name = "opentern-x86_64-unknown-linux-gnu.tar.gz";
        let t = std::time::Instant::now();
        download_with(
            &format!("{base}/{name}"),
            &dir.path().join(name),
            &Limits::NORMAL,
            &|_| {},
            &|| eprintln!("retrying at {:?}", t.elapsed()),
        )
        .unwrap();
        download(&format!("{base}/SHA256SUMS"), &dir.path().join("SUMS"), &|_| {}).unwrap();
        let sums = std::fs::read_to_string(dir.path().join("SUMS")).unwrap();
        let want = sums
            .lines()
            .find(|l| l.ends_with(name))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        assert_eq!(sha256_file(&dir.path().join(name)).unwrap(), want);
        eprintln!("{name} in {:?}", t.elapsed());
    }

    #[test]
    fn asset_names_match_release_builds() {
        // A contract with release.yml, install.sh and install.ps1.
        let known = [
            "opentern-x86_64-unknown-linux-gnu.tar.gz",
            "opentern-aarch64-unknown-linux-gnu.tar.gz",
            "Opentern-macos-universal.zip",
            "Opentern-windows-x64.zip",
            "Opentern-windows-arm64.zip",
        ];
        let name = asset_name().expect("CI platforms have a release build");
        assert!(known.contains(&name.as_str()), "{name}");
    }

    #[test]
    fn shown_drops_verbatim_prefix() {
        assert_eq!(
            shown(Path::new(r"\\?\C:\Program Files\Opentern")),
            r"C:\Program Files\Opentern"
        );
        assert_eq!(shown(Path::new("/usr/bin")), "/usr/bin");
    }

    /// A zip laid out like the Windows release (exe at the root, docs next
    /// to it) installs over the target, on every platform.
    #[test]
    fn install_from_zip() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("Opentern-windows-x64.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in [
            ("README.md", "docs"),
            ("opentern.exe", "new build"),
            ("LICENSE.txt", "MIT"),
        ] {
            w.start_file(name, opts).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap();

        let unpacked = dir.path().join("unpacked");
        std::fs::create_dir_all(&unpacked).unwrap();
        unzip(&zip_path, &unpacked).unwrap();
        let fresh = find(&unpacked, std::ffi::OsStr::new("opentern.exe")).unwrap();
        let target = dir.path().join("opentern.exe");
        std::fs::write(&target, "old build").unwrap();
        swap(&fresh, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new build");

        // A corrupt download fails cleanly instead of panicking.
        let bad = dir.path().join("bad.zip");
        std::fs::write(&bad, "not a zip").unwrap();
        assert!(unzip(&bad, &unpacked).is_err());
    }

    /// Windows won't delete a running exe but will rename it: the update
    /// swaps in the new one while the old keeps running, and the leftover is
    /// removed on the next launch.
    #[cfg(windows)]
    #[test]
    fn swap_replaces_a_running_exe() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("opentern.exe");
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        std::fs::copy(Path::new(&system).join(r"System32\cmd.exe"), &target).unwrap();
        let mut running = std::process::Command::new(&target)
            .args(["/c", "ping", "-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        // Deleting it really is refused while it runs.
        assert!(std::fs::remove_file(&target).is_err());

        let fresh = dir.path().join("fresh.exe");
        std::fs::write(&fresh, "new build").unwrap();
        let r = swap(&fresh, &target);
        let _ = running.kill();
        let _ = running.wait();
        r.unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new build");
    }
}
