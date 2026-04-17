//! WhatsApp sending via Baileys (Node.js subprocess).
//!
//! The Node script lives in `whatsapp/send.mjs` and expects
//! `whatsapp/node_modules` to be installed (`npm install` in that directory).
//! The WhatsApp session (QR scan) persists in `whatsapp/auth/`.
//!
//! First-run pairing works fully from the GUI: `send.mjs` prints a
//! `__QR__:<raw-data>` sentinel line that Rust parses and renders natively.

use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

/// Event streamed from the Node subprocess.
pub enum WhatsappEvent {
    /// Regular log line from the Node script.
    Line(String),
    /// Raw QR data (Baileys string). Render with the `qrcode` crate.
    Qr(String),
}

/// Locate the `node` binary by checking common install paths, falling back to PATH.
pub fn find_node() -> String {
    const CANDIDATES: &[&str] = &[
        "/opt/homebrew/bin/node",
        "/usr/local/bin/node",
        "/opt/homebrew/opt/node/bin/node",
        "/usr/bin/node",
        // Windows default (nvm-windows / official installer)
        "C:\\Program Files\\nodejs\\node.exe",
    ];
    for p in CANDIDATES {
        if Path::new(p).exists() {
            return (*p).to_string();
        }
    }
    // Check any nvm install under $HOME.
    if let Some(home) = std::env::var_os("HOME") {
        let nvm = PathBuf::from(home).join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(&nvm) {
            let mut versions: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            versions.sort();
            if let Some(latest) = versions.last() {
                let node = latest.join("bin/node");
                if node.exists() {
                    return node.to_string_lossy().into_owned();
                }
            }
        }
    }
    "node".to_string()
}

/// Locate the `whatsapp/` script directory.
/// Checks (in order): CARGO_MANIFEST_DIR (dev), cwd, executable dir.
pub fn find_script_dir() -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Dev build: baked at compile time.
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("whatsapp"));

    // Current working directory.
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("whatsapp"));
    }

    // Next to the executable (packaged builds).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("whatsapp"));
            // macOS .app: Contents/MacOS/bin -> Contents/Resources/whatsapp
            if let Some(grandparent) = parent.parent() {
                candidates.push(grandparent.join("Resources/whatsapp"));
            }
        }
    }

    for c in &candidates {
        if c.join("send.mjs").exists() {
            return Ok(c.clone());
        }
    }

    bail!(
        "whatsapp/send.mjs not found. Searched: {:?}",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    )
}

/// Run a Node script in `whatsapp/` and stream events to the caller.
/// `on_event` is invoked synchronously from reader threads as lines arrive.
/// Returns Ok on exit-status success; bails with combined output otherwise.
fn run_node_streaming<F>(args: &[&str], on_event: F) -> Result<()>
where
    F: Fn(WhatsappEvent) + Send + Sync + 'static,
{
    let script_dir = find_script_dir()?;
    if !script_dir.join("node_modules").exists() {
        bail!(
            "{} is missing. Run `npm install` in {}",
            script_dir.join("node_modules").display(),
            script_dir.display()
        );
    }

    let node = find_node();
    let on_event = std::sync::Arc::new(on_event);
    on_event(WhatsappEvent::Line(format!(
        "spawning: {} {} (in {})",
        node,
        args.join(" "),
        script_dir.display()
    )));

    let mut child = Command::new(&node)
        .args(args)
        .current_dir(&script_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn node ({})", node))?;

    let stdout = child.stdout.take().context("no stdout")?;
    let stderr = child.stderr.take().context("no stderr")?;

    // Collect all lines so we can surface them on non-zero exit.
    let collected = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

    fn pump<R: std::io::Read + Send + 'static, F: Fn(WhatsappEvent) + Send + Sync + 'static>(
        r: R,
        cb: std::sync::Arc<F>,
        col: std::sync::Arc<std::sync::Mutex<String>>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            for line in BufReader::new(r).lines().map_while(|l| l.ok()) {
                if let Some(qr) = line.strip_prefix("__QR__:") {
                    cb(WhatsappEvent::Qr(qr.to_string()));
                } else {
                    if let Ok(mut g) = col.lock() {
                        g.push_str(&line);
                        g.push('\n');
                    }
                    cb(WhatsappEvent::Line(line));
                }
            }
        })
    }

    let t1 = pump(stdout, on_event.clone(), collected.clone());
    let t2 = pump(stderr, on_event.clone(), collected.clone());

    let status = child.wait().context("wait on node child failed")?;
    let _ = t1.join();
    let _ = t2.join();

    if !status.success() {
        let out = collected.lock().map(|g| g.clone()).unwrap_or_default();
        bail!("WhatsApp node exited {:?}:\n{}", status.code(), out.trim());
    }
    Ok(())
}

/// Normalise a human-entered WhatsApp recipient into the canonical JID form.
///
/// Accepts:
///   * Full JIDs (`…@g.us`, `…@s.whatsapp.net`, `…@c.us`) — passed through unchanged.
///   * Pure numeric group IDs (`120363…`) — passed through; Node decides suffix by length.
///   * Phone numbers with spaces / `+` / parens / dots (e.g. `+41 79 236 45 44`) —
///     stripped to digits (and a single leading hyphen for legacy groups).
pub fn normalize_jid(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.contains('@') {
        return trimmed.to_string();
    }
    // Keep digits and a single '-' (legacy group IDs use `<digits>-<digits>`).
    trimmed
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '-')
        .collect()
}

/// Send a file (image/PDF/HTML/…) to a WhatsApp JID, streaming events.
/// The `jid` argument is normalised first so phone numbers like
/// `+41 79 236 45 44` are accepted verbatim.
pub fn send_streaming<F>(jid: &str, file: &str, caption: &str, on_event: F) -> Result<()>
where
    F: Fn(WhatsappEvent) + Send + Sync + 'static,
{
    let abs = std::fs::canonicalize(file).with_context(|| format!("File not found: {}", file))?;
    let abs_s = abs.to_string_lossy().into_owned();
    let normalized = normalize_jid(jid);
    if normalized.is_empty() {
        anyhow::bail!("WhatsApp recipient is empty after normalisation: {:?}", jid);
    }
    run_node_streaming(&["send.mjs", &normalized, &abs_s, caption], on_event)
}

/// Convenience wrapper: blocks, collects all lines, returns them on success.
pub fn send(jid: &str, file: &str, caption: &str) -> Result<String> {
    let buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let buf_cb = buf.clone();
    send_streaming(jid, file, caption, move |ev| {
        if let WhatsappEvent::Line(l) = ev {
            if let Ok(mut g) = buf_cb.lock() {
                g.push_str(&l);
                g.push('\n');
            }
        }
    })?;
    Ok(buf.lock().map(|g| g.clone()).unwrap_or_default())
}

/// List joined WhatsApp groups, streaming events.
pub fn list_groups_streaming<F>(on_event: F) -> Result<()>
where
    F: Fn(WhatsappEvent) + Send + Sync + 'static,
{
    run_node_streaming(&["list-groups.mjs"], on_event)
}

/// List 1:1 contacts (and chats) known to this session, optionally filtered.
pub fn list_contacts_streaming<F>(filter: Option<&str>, on_event: F) -> Result<()>
where
    F: Fn(WhatsappEvent) + Send + Sync + 'static,
{
    let mut args: Vec<&str> = vec!["list-contacts.mjs"];
    if let Some(f) = filter {
        args.push(f);
    }
    run_node_streaming(&args, on_event)
}
