//! Diff metrics from real git status / unified diffs / tree comparison.
//!
//! Known MVP limits (documented intentionally):
//! - Dependency heuristics may over-count non-dependency key edits in manifests.
//! - Porcelain without `-z` can mis-handle unusual paths; we prefer `-z` when available.
//! - Binary unified-diff hunks are classified as file changes without line counts.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, TifError};
use crate::scoring::DiffMetrics;

/// Collect working-tree metrics from a Git repository.
///
/// Uses `git status -z` (NUL-terminated) when available, falling back to
/// `--porcelain=v1` line mode. Line stats come from a single
/// `git diff --numstat HEAD` plus untracked file line counts.
pub fn metrics_from_git(repo_root: &Path) -> Result<DiffMetrics> {
    let status_z = git_output(repo_root, &["status", "-z", "-uall"]);
    let entries = match status_z {
        Ok(raw) if !raw.is_empty() || status_looks_empty_ok(&raw) => parse_status_z(&raw),
        _ => {
            let status = git_output(repo_root, &["status", "--porcelain=v1", "-uall"])?;
            parse_status_porcelain_v1(&status)
        }
    };

    let mut metrics = DiffMetrics::default();
    let mut paths: Vec<String> = Vec::new();

    for (xy, path) in entries {
        if path.is_empty() {
            continue;
        }
        let x = xy.chars().next().unwrap_or(' ');
        let y = xy.chars().nth(1).unwrap_or(' ');

        if x == '?' && y == '?' {
            metrics.files_added += 1;
            if is_test_path(&path) {
                metrics.tests_changed += 1;
            }
            let full = repo_root.join(&path);
            if full.is_file() {
                if let Ok(text) = fs::read_to_string(&full) {
                    metrics.lines_added += text.lines().count() as u32;
                }
            }
        } else if x == 'A' || y == 'A' {
            metrics.files_added += 1;
            if is_test_path(&path) {
                metrics.tests_changed += 1;
            }
        } else if x == 'D' || y == 'D' {
            metrics.files_deleted += 1;
            if is_test_path(&path) {
                metrics.tests_changed += 1;
            }
        } else if x != ' ' || y != ' ' {
            // Modified, renamed, copied, typechange, etc.
            metrics.files_changed += 1;
            if is_test_path(&path) {
                metrics.tests_changed += 1;
            }
        }

        if looks_like_dependency_manifest(&path) && (x == 'A' || y == 'A' || x == 'M' || y == 'M') {
            metrics.configuration_surface_added =
                metrics.configuration_surface_added.saturating_add(1);
        }

        if !paths.contains(&path) {
            paths.push(path);
        }
    }

    // Single coherent view: working tree + index vs HEAD.
    let numstat = git_output(repo_root, &["diff", "--numstat", "HEAD"]).unwrap_or_default();
    let (add, del) = parse_numstat_totals(&numstat);
    metrics.lines_added = metrics.lines_added.saturating_add(add);
    metrics.lines_removed = metrics.lines_removed.saturating_add(del);

    metrics.runtime_dependencies_added = estimate_runtime_deps_added(repo_root, &paths);
    metrics.changed_paths = paths;
    Ok(metrics)
}

fn status_looks_empty_ok(raw: &str) -> bool {
    raw.is_empty()
}

/// Parse `git status -z` records.
/// Format: `XY path\0` or for renames `XY old\0new\0` (git uses two NULs with rename).
/// Actually: `XY path\0` and for rename `R  old -> new` is not used with -z;
/// with -z rename is `XY\0old\0new\0` in some versions, or `XY old\0new\0`.
fn parse_status_z(raw: &str) -> Vec<(String, String)> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if i + 3 > bytes.len() {
            break;
        }
        // XY + space
        let xy = String::from_utf8_lossy(&bytes[i..i + 2]).into_owned();
        i += 2;
        if i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        // path until NUL
        let start = i;
        while i < bytes.len() && bytes[i] != 0 {
            i += 1;
        }
        let path1 = String::from_utf8_lossy(&bytes[start..i]).replace('\\', "/");
        if i < bytes.len() && bytes[i] == 0 {
            i += 1;
        }
        // Rename/copy: second path follows immediately.
        let path = if matches!(xy.chars().next(), Some('R' | 'C')) && i < bytes.len() {
            let start2 = i;
            while i < bytes.len() && bytes[i] != 0 {
                i += 1;
            }
            let path2 = String::from_utf8_lossy(&bytes[start2..i]).replace('\\', "/");
            if i < bytes.len() && bytes[i] == 0 {
                i += 1;
            }
            if path2.is_empty() {
                path1
            } else {
                path2
            }
        } else {
            path1
        };
        if !path.is_empty() {
            out.push((xy, path));
        }
    }
    out
}

/// Line-mode porcelain v1: `XY path` or `XY old -> new` for renames.
/// Paths containing ` -> ` as a substring are imperfectly handled in line mode;
/// prefer `-z` when available.
fn parse_status_porcelain_v1(status: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in status.lines() {
        if line.len() < 3 {
            continue;
        }
        let xy = line[..2].to_string();
        let path_part = line[3..].trim();
        // Git rename line format ends with ` -> newpath` after the old path.
        // Prefer rsplit once so a path that itself contains " -> " is less wrong.
        let path = if xy.starts_with('R') || xy.contains('R') {
            path_part
                .rsplit_once(" -> ")
                .map(|(_, new)| new.trim())
                .unwrap_or(path_part)
        } else if let Some((_, new)) = path_part.rsplit_once(" -> ") {
            // Some versions put R in second column.
            new.trim()
        } else {
            path_part
        };
        let path = path.trim_matches('"').replace('\\', "/");
        if !path.is_empty() {
            out.push((xy, path));
        }
    }
    out
}

/// Absolute simplicity-oriented metrics for a full tree (not a delta).
///
/// Use this for Firebreak ranking when git metrics are unavailable so original
/// and candidate are measured with the **same** quantity:
/// - `files_added` = total file count in the tree
/// - `lines_added` = total line count across text files
/// - `changed_paths` = all relative paths
///
/// Do **not** use [`metrics_from_tree_diff`] for ranking against git-style
/// original metrics — that measures source↔candidate deltas and can score an
/// identical tree near zero (fail-open).
pub fn metrics_from_tree_absolute(root: &Path) -> Result<DiffMetrics> {
    let files = collect_tree_files(root)?;
    let mut metrics = DiffMetrics::default();
    let mut paths = Vec::new();
    let mut total_lines = 0u32;
    let mut config_hits = 0u32;
    let mut test_hits = 0u32;

    for (rel, path) in &files {
        paths.push(rel.clone());
        if is_test_path(rel) {
            test_hits = test_hits.saturating_add(1);
        }
        if looks_like_dependency_manifest(rel) {
            config_hits = config_hits.saturating_add(1);
        }
        if let Ok(text) = fs::read_to_string(path) {
            // Binary-ish: skip huge non-utf8 by using lossy only when utf8 fails
            total_lines = total_lines.saturating_add(text.lines().count() as u32);
        } else if let Ok(bytes) = fs::read(path) {
            // Non-UTF8: count as one "line" unit so binary blobs still cost something.
            if !bytes.is_empty() {
                total_lines = total_lines.saturating_add(1);
            }
        }
    }

    // Map absolute weight into the fields SimplicityScorer already understands.
    // files_added ≈ tree size; lines_added ≈ total lines; no invented deps.
    metrics.files_added = files.len() as u32;
    metrics.lines_added = total_lines;
    metrics.tests_changed = test_hits;
    metrics.configuration_surface_added = config_hits;
    metrics.changed_paths = paths;
    Ok(metrics)
}

/// Compare two directory trees (original vs candidate) without git.
///
/// Returns a **delta** (only in candidate / only in original / content differs).
/// Suitable for Damage Assessment path lists — **not** for Firebreak ranking
/// against absolute or git-style original metrics. Use
/// [`metrics_from_tree_absolute`] on each side for comparable ranking scores.
pub fn metrics_from_tree_diff(original_root: &Path, candidate_root: &Path) -> Result<DiffMetrics> {
    let orig = collect_tree_files(original_root)?;
    let cand = collect_tree_files(candidate_root)?;

    let mut metrics = DiffMetrics::default();
    let mut paths = Vec::new();

    for (rel, cand_path) in &cand {
        match orig.get(rel) {
            None => {
                metrics.files_added += 1;
                if let Ok(text) = fs::read_to_string(cand_path) {
                    metrics.lines_added += text.lines().count() as u32;
                }
                if is_test_path(rel) {
                    metrics.tests_changed += 1;
                }
                paths.push(rel.clone());
            }
            Some(orig_path) => {
                let o = fs::read(orig_path).unwrap_or_default();
                let c = fs::read(cand_path).unwrap_or_default();
                if o != c {
                    metrics.files_changed += 1;
                    let (add, del) = line_diff_counts(
                        &String::from_utf8_lossy(&o),
                        &String::from_utf8_lossy(&c),
                    );
                    metrics.lines_added = metrics.lines_added.saturating_add(add);
                    metrics.lines_removed = metrics.lines_removed.saturating_add(del);
                    if is_test_path(rel) {
                        metrics.tests_changed += 1;
                    }
                    if looks_like_dependency_manifest(rel) {
                        metrics.configuration_surface_added =
                            metrics.configuration_surface_added.saturating_add(1);
                        // Tree mode: do not invent runtime dep counts without lockfile parse.
                    }
                    paths.push(rel.clone());
                }
            }
        }
    }
    for rel in orig.keys() {
        if !cand.contains_key(rel) {
            metrics.files_deleted += 1;
            if let Some(p) = orig.get(rel) {
                if let Ok(text) = fs::read_to_string(p) {
                    metrics.lines_removed += text.lines().count() as u32;
                }
            }
            if is_test_path(rel) {
                metrics.tests_changed += 1;
            }
            paths.push(rel.clone());
        }
    }

    metrics.changed_paths = paths;
    Ok(metrics)
}

fn collect_tree_files(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut map = BTreeMap::new();
    walk_tree(root, root, &mut map)?;
    Ok(map)
}

fn walk_tree(root: &Path, current: &Path, out: &mut BTreeMap<String, PathBuf>) -> Result<()> {
    if !current.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(
            name_str.as_ref(),
            ".git" | ".this-is-fine" | "target" | "node_modules" | ".venv" | "dist" | "build"
        ) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk_tree(root, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, path);
        }
    }
    Ok(())
}

/// Rough line add/del counts without a full diff algorithm.
fn line_diff_counts(old: &str, new: &str) -> (u32, u32) {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    // Multiset-ish: count lines only in new as adds, only in old as dels.
    let mut old_counts: BTreeMap<&str, u32> = BTreeMap::new();
    for l in &old_lines {
        *old_counts.entry(*l).or_default() += 1;
    }
    let mut new_counts: BTreeMap<&str, u32> = BTreeMap::new();
    for l in &new_lines {
        *new_counts.entry(*l).or_default() += 1;
    }
    let mut add = 0u32;
    let mut del = 0u32;
    for (l, n) in &new_counts {
        let o = old_counts.get(l).copied().unwrap_or(0);
        if *n > o {
            add += *n - o;
        }
    }
    for (l, o) in &old_counts {
        let n = new_counts.get(l).copied().unwrap_or(0);
        if *o > n {
            del += *o - n;
        }
    }
    // Fallback if both empty of unique lines but lengths differ.
    if add == 0 && del == 0 && old_lines.len() != new_lines.len() {
        if new_lines.len() > old_lines.len() {
            add = (new_lines.len() - old_lines.len()) as u32;
        } else {
            del = (old_lines.len() - new_lines.len()) as u32;
        }
    }
    (add, del)
}

/// Parse a unified diff text into rough `DiffMetrics`.
pub fn metrics_from_unified_diff(diff: &str) -> DiffMetrics {
    let mut metrics = DiffMetrics::default();
    let mut current_path: Option<String> = None;
    let mut file_is_new = false;
    let mut file_is_deleted = false;
    let mut binary_file = false;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            if let Some(ref p) = current_path {
                classify_file(&mut metrics, p, file_is_new, file_is_deleted, binary_file);
            }
            current_path = None;
            file_is_new = false;
            file_is_deleted = false;
            binary_file = false;
            if let Some(rest) = line.strip_prefix("diff --git ") {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() >= 2 {
                    let p = parts[1].trim_start_matches("b/").replace('\\', "/");
                    current_path = Some(p);
                }
            }
        } else if line.starts_with("new file mode") {
            file_is_new = true;
        } else if line.starts_with("deleted file mode") {
            file_is_deleted = true;
        } else if line.starts_with("Binary files ") && line.contains(" differ") {
            binary_file = true;
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            let p = rest
                .trim()
                .trim_start_matches("b/")
                .split('\t')
                .next()
                .unwrap_or("")
                .replace('\\', "/");
            if p != "/dev/null" && !p.is_empty() {
                current_path = Some(p);
            }
        } else if let Some(rest) = line.strip_prefix("--- ") {
            let p = rest
                .trim()
                .trim_start_matches("a/")
                .split('\t')
                .next()
                .unwrap_or("")
                .replace('\\', "/");
            if p == "/dev/null" {
                file_is_new = true;
            }
        } else if !binary_file && line.starts_with('+') && !line.starts_with("+++") {
            metrics.lines_added += 1;
        } else if !binary_file && line.starts_with('-') && !line.starts_with("---") {
            metrics.lines_removed += 1;
        }
    }
    if let Some(ref p) = current_path {
        classify_file(&mut metrics, p, file_is_new, file_is_deleted, binary_file);
    }

    if metrics.changed_paths.is_empty()
        && (metrics.lines_added > 0 || metrics.lines_removed > 0)
        && metrics.files_added == 0
        && metrics.files_changed == 0
        && metrics.files_deleted == 0
    {
        metrics.files_changed = 1;
    }

    metrics
}

fn classify_file(
    metrics: &mut DiffMetrics,
    path: &str,
    is_new: bool,
    is_deleted: bool,
    _binary: bool,
) {
    let norm = path.replace('\\', "/");
    if !metrics.changed_paths.contains(&norm) {
        metrics.changed_paths.push(norm.clone());
    }
    if is_new {
        metrics.files_added += 1;
    } else if is_deleted {
        metrics.files_deleted += 1;
    } else {
        metrics.files_changed += 1;
    }
    if is_test_path(&norm) {
        metrics.tests_changed += 1;
    }
    if looks_like_dependency_manifest(&norm) {
        metrics.configuration_surface_added = metrics.configuration_surface_added.saturating_add(1);
    }
}

fn parse_numstat_totals(numstat: &str) -> (u32, u32) {
    let mut add = 0u32;
    let mut del = 0u32;
    for line in numstat.lines() {
        let mut parts = line.split_whitespace();
        let a = parts.next().unwrap_or("0");
        let d = parts.next().unwrap_or("0");
        if a != "-" {
            if let Ok(n) = a.parse::<u32>() {
                add = add.saturating_add(n);
            }
        }
        if d != "-" {
            if let Ok(n) = d.parse::<u32>() {
                del = del.saturating_add(n);
            }
        }
    }
    (add, del)
}

fn is_test_path(path: &str) -> bool {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    p.contains("/test/")
        || p.contains("/tests/")
        || p.contains("__tests__")
        || p.ends_with("_test.rs")
        || p.ends_with("_test.go")
        || p.ends_with(".test.ts")
        || p.ends_with(".test.js")
        || p.ends_with(".spec.ts")
        || p.ends_with(".spec.js")
        || p.ends_with("_test.py")
        || p.starts_with("test_")
}

fn looks_like_dependency_manifest(path: &str) -> bool {
    let name = path.replace('\\', "/");
    let base = name.rsplit('/').next().unwrap_or(&name);
    matches!(
        base,
        "Cargo.toml"
            | "Cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "go.mod"
            | "go.sum"
            | "requirements.txt"
            | "Pipfile"
            | "Pipfile.lock"
            | "pyproject.toml"
            | "Gemfile"
            | "Gemfile.lock"
            | "composer.json"
            | "composer.lock"
    )
}

/// Heuristic: count added dependency lines in common manifests (may over-count).
fn estimate_runtime_deps_added(repo_root: &Path, paths: &[String]) -> u32 {
    let mut count = 0u32;
    for p in paths {
        if !looks_like_dependency_manifest(p) {
            continue;
        }
        let base = p.rsplit('/').next().unwrap_or(p);
        if let Ok(diff) = git_output(repo_root, &["diff", "HEAD", "--", p]) {
            count = count.saturating_add(count_dep_additions(base, &diff));
        } else if let Ok(diff) = git_output(repo_root, &["diff", "--", p]) {
            count = count.saturating_add(count_dep_additions(base, &diff));
        }
    }
    count
}

fn count_dep_additions(filename: &str, diff: &str) -> u32 {
    let mut n = 0u32;
    let mut in_deps_section = false;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            let body = &line[1..];
            let trimmed = body.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }
            match filename {
                "Cargo.toml" => {
                    if trimmed.starts_with('[') {
                        in_deps_section = trimmed.contains("dependencies");
                        continue;
                    }
                    // Only count package-like keys inside dependency tables.
                    if in_deps_section && trimmed.contains('=') && !trimmed.starts_with('[') {
                        n += 1;
                    }
                }
                "package.json" => {
                    // Skip pure metadata keys common outside dependencies.
                    if trimmed.starts_with("\"name\"")
                        || trimmed.starts_with("\"version\"")
                        || trimmed.starts_with("\"description\"")
                        || trimmed.starts_with("\"scripts\"")
                        || trimmed.starts_with("\"main\"")
                        || trimmed.starts_with("\"license\"")
                    {
                        continue;
                    }
                    if trimmed.contains(':')
                        && !trimmed.starts_with('{')
                        && !trimmed.starts_with('}')
                    {
                        n += 1;
                    }
                }
                "requirements.txt" | "go.mod" if !trimmed.is_empty() => {
                    n += 1;
                }
                _ => {}
            }
        } else if line.starts_with('-') && !line.starts_with("---") {
            // Track section from context-less minus lines poorly; ignore.
        } else if filename == "Cargo.toml" {
            let t = line.trim_start_matches([' ', '+', '-']).trim();
            if t.starts_with('[') {
                in_deps_section = t.contains("dependencies");
            }
        }
    }
    n
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| TifError::Other(format!("git invocation failed: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TifError::Other(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn parse_unified_diff_basic() {
        let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 fn a() {}
+fn b() {}
 fn c() {}
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
index 000..333
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,2 @@
+pub fn n() {}
+// ok
diff --git a/old.rs b/old.rs
deleted file mode 100644
index 444..000
--- a/old.rs
+++ /dev/null
@@ -1 +0,0 @@
-gone
"#;
        let m = metrics_from_unified_diff(diff);
        assert_eq!(m.files_added, 1);
        assert_eq!(m.files_deleted, 1);
        assert_eq!(m.files_changed, 1);
        assert!(m.lines_added >= 3);
        assert!(m.lines_removed >= 1);
        assert!(m.changed_paths.iter().any(|p| p.contains("new.rs")));
    }

    #[test]
    fn parse_unified_diff_binary_header() {
        let diff = r#"diff --git a/img.png b/img.png
index 111..222
Binary files a/img.png and b/img.png differ
"#;
        let m = metrics_from_unified_diff(diff);
        assert_eq!(m.files_changed, 1);
        assert!(m.changed_paths.iter().any(|p| p.contains("img.png")));
    }

    #[test]
    fn parse_rename_via_rsplit() {
        let status = "R  old name -> new name.txt\n";
        let entries = parse_status_porcelain_v1(status);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].1.contains("new name.txt"));
    }

    #[test]
    fn tree_diff_counts_add_change_delete() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("keep.txt"), "same\n").unwrap();
        fs::write(b.join("keep.txt"), "same\n").unwrap();
        fs::write(a.join("change.txt"), "v1\n").unwrap();
        fs::write(b.join("change.txt"), "v2\n").unwrap();
        fs::write(a.join("gone.txt"), "x\n").unwrap();
        fs::write(b.join("new.txt"), "y\n").unwrap();
        let m = metrics_from_tree_diff(&a, &b).unwrap();
        assert_eq!(m.files_added, 1);
        assert_eq!(m.files_deleted, 1);
        assert_eq!(m.files_changed, 1);
    }

    #[test]
    fn tree_absolute_identical_trees_have_equal_weight() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("f.txt"), "one\ntwo\n").unwrap();
        fs::write(b.join("f.txt"), "one\ntwo\n").unwrap();
        let ma = metrics_from_tree_absolute(&a).unwrap();
        let mb = metrics_from_tree_absolute(&b).unwrap();
        assert_eq!(ma.files_added, mb.files_added);
        assert_eq!(ma.lines_added, mb.lines_added);
        assert!(ma.lines_added >= 2);
    }

    #[test]
    fn tree_absolute_larger_tree_has_higher_weight() {
        let dir = tempdir().unwrap();
        let small = dir.path().join("small");
        let large = dir.path().join("large");
        fs::create_dir_all(&small).unwrap();
        fs::create_dir_all(&large).unwrap();
        fs::write(small.join("a.txt"), "x\n").unwrap();
        fs::write(large.join("a.txt"), "x\n").unwrap();
        fs::write(large.join("b.txt"), "y\nz\n").unwrap();
        let ms = metrics_from_tree_absolute(&small).unwrap();
        let ml = metrics_from_tree_absolute(&large).unwrap();
        assert!(ml.files_added > ms.files_added || ml.lines_added > ms.lines_added);
    }

    #[test]
    fn tree_diff_identical_is_near_zero_unlike_absolute() {
        // Documents why tree-diff must not be used for Firebreak ranking.
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("big.txt"), "line\n".repeat(50)).unwrap();
        fs::write(b.join("big.txt"), "line\n".repeat(50)).unwrap();
        let delta = metrics_from_tree_diff(&a, &b).unwrap();
        let abs = metrics_from_tree_absolute(&b).unwrap();
        assert_eq!(delta.files_added, 0);
        assert_eq!(delta.files_changed, 0);
        assert_eq!(delta.lines_added, 0);
        assert!(abs.lines_added >= 50);
    }

    #[test]
    fn metrics_from_git_repo() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let ok = Command::new("git")
            .args(["init"])
            .current_dir(root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let _ = Command::new("git")
            .args(["config", "user.email", "tif@test.local"])
            .current_dir(root)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.name", "tif"])
            .current_dir(root)
            .status();
        fs::write(root.join("a.txt"), "one\n").unwrap();
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .status();
        let commit_ok = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !commit_ok {
            return;
        }

        fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        fs::write(root.join("b.txt"), "new\nfile\n").unwrap();

        let m = metrics_from_git(root).unwrap();
        assert!(m.files_added >= 1, "expected untracked file: {m:?}");
        assert!(m.files_changed >= 1 || m.lines_added >= 1, "{m:?}");
        assert!(m.changed_paths.iter().any(|p| p.contains("b.txt")));
    }
}
