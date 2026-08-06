//! Pure, side-effect-free helpers shared by the built-in framework
//! drivers.
//!
//! These never spawn a process or mutate the filesystem beyond reading for
//! a comparison, so they are safe to call from `plan`/`status`/`prepare`
//! paths. The Cosh/Codex/Claude Code drivers share them here rather than
//! each re-declaring the same comparison/timestamp/formatting logic.

use std::path::{Path, PathBuf};

use super::driver::{CliOutput, ConditionStatus, FrameworkCommand};

/// Relative paths of every regular file under `root`, sorted, skipping
/// top-level entries named in `exclude_top`. The exclusion models delivery
/// projections: a driver that deliberately copies only part of the bundle
/// (hermes leaves `skills/` to separate skill delivery) records and
/// compares exactly the projection it copies.
pub(crate) fn delivered_files(
    root: &Path,
    exclude_top: &[&str],
) -> std::io::Result<Vec<PathBuf>> {
    fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                collect_files(&path, out)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if exclude_top
            .iter()
            .any(|skip| entry.file_name() == std::ffi::OsStr::new(skip))
        {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(&path, &mut files)?;
        } else {
            files.push(path);
        }
    }
    let mut rels: Vec<PathBuf> = files
        .into_iter()
        .map(|path| path.strip_prefix(root).unwrap_or(&path).to_path_buf())
        .collect();
    rels.sort();
    Ok(rels)
}

/// Compare a delivered source tree against an installed copy with subset
/// semantics: every regular file under `source` (minus `exclude_top`
/// projections) must have a byte-identical counterpart at the same
/// relative path under `copy`. Extra files in the copy are ignored — the
/// copy is the executed artifact and legitimately accretes state the
/// delivery never shipped (runtime-derived files such as Python's
/// `__pycache__`, ownership markers). The removal direction — a file the
/// delivery no longer ships but the copy still carries — cannot be decided
/// from these two trees alone and is covered by
/// [`lingering_removed_files`] against the enable-time record.
///
/// Returns the sorted relative paths that are missing from or differ in
/// the copy (empty means the copy matches the delivery). Returns `Err`
/// when `source` cannot be walked or read, so callers report `Unknown`
/// rather than a wrong verdict. Symlinked entries compare their referent
/// bytes (reads follow links); directories contribute only their files.
pub(crate) fn copy_divergence(
    source: &Path,
    copy: &Path,
    exclude_top: &[&str],
) -> std::io::Result<Vec<PathBuf>> {
    let mut diverged = Vec::new();
    for rel in delivered_files(source, exclude_top)? {
        let delivered = std::fs::read(source.join(&rel))?;
        // A missing or unreadable counterpart is divergence, not Unknown:
        // the delivery side read fine, so the verdict "the copy does not
        // carry this delivered file" is decidable.
        match std::fs::read(copy.join(&rel)) {
            Ok(installed) if installed == delivered => {}
            _ => diverged.push(rel),
        }
    }
    Ok(diverged)
}

/// Enable-time delivered paths that the current delivery no longer ships
/// but the executed copy still carries — the removal direction of copy
/// staleness (a same-version release dropping a file leaves the copy
/// executing it until re-enable replaces the tree). `recorded` is the
/// receipt's enable-time file list; an empty list (receipt written before
/// recording existed) detects nothing rather than everything.
pub(crate) fn lingering_removed_files(
    recorded: &[String],
    source: &Path,
    copy: &Path,
) -> Vec<PathBuf> {
    let mut lingering: Vec<PathBuf> = recorded
        .iter()
        .map(PathBuf::from)
        .filter(|rel| !source.join(rel).exists() && copy.join(rel).exists())
        .collect();
    lingering.sort();
    lingering
}

/// Human-readable list of diverged relative paths for a condition reason:
/// up to three named, the rest counted.
pub(crate) fn display_paths(paths: &[PathBuf]) -> String {
    let named: Vec<String> = paths
        .iter()
        .take(3)
        .map(|p| p.display().to_string())
        .collect();
    let mut out = named.join(", ");
    if paths.len() > 3 {
        out.push_str(&format!(" and {} more", paths.len() - 3));
    }
    out
}

/// ISO 8601 UTC timestamp, second precision.
pub(crate) fn now_iso8601() -> String {
    use chrono::{SecondsFormat, Utc};
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Map a bool to a [`ConditionStatus`] (`true` -> `True`, `false` -> `False`).
pub(crate) fn bool_status(b: bool) -> ConditionStatus {
    if b {
        ConditionStatus::True
    } else {
        ConditionStatus::False
    }
}

/// Compose a failure reason string from a non-success [`CliOutput`].
pub(crate) fn cli_failure_reason(verb: &str, output: &CliOutput) -> String {
    if output.timed_out {
        return format!("'{verb}' timed out");
    }
    let code = output
        .status
        .map(|c| c.to_string())
        .unwrap_or_else(|| "killed".to_string());
    let mut reason = format!("'{verb}' exited with {code}");
    let stderr = output.stderr.trim();
    if !stderr.is_empty() {
        reason.push_str(": ");
        reason.push_str(stderr);
    }
    reason
}

/// Human-readable form of a command for dry-run/preview output. Display
/// only — never parsed back into an argv.
pub(crate) fn display_command(cmd: &FrameworkCommand) -> String {
    let mut s = String::new();
    for (k, v) in &cmd.env_set {
        s.push_str(&format!("{k}={v} "));
    }
    s.push_str(&cmd.program);
    for a in &cmd.args {
        s.push(' ');
        s.push_str(a);
    }
    s
}
