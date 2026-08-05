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

/// Maximum accepted unified-diff text size (64 MiB).
pub const MAX_UNIFIED_DIFF_BYTES: usize = 64 * 1024 * 1024;

/// Soft bound on path/line map entries to avoid unbounded memory growth.
const MAX_PATH_MAP_ENTRIES: usize = 500_000;

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
    // Prefer structured dependency deltas when manifests are readable.
    if let Ok(delta) = dependency_delta_from_git(repo_root, &paths) {
        if delta.added_count() > 0 {
            metrics.runtime_dependencies_added = delta.added_count();
        }
    }
    apply_generated_code_heuristics(&mut metrics, Some(repo_root));
    metrics.changed_paths = paths;
    Ok(metrics)
}

/// Named dependency delta from Cargo.toml / package.json comparisons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencyDelta {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl DependencyDelta {
    pub fn added_count(&self) -> u32 {
        self.added.len() as u32
    }

    pub fn removed_count(&self) -> u32 {
        self.removed.len() as u32
    }

    pub fn merge(&mut self, other: DependencyDelta) {
        for a in other.added {
            if !self.added.contains(&a) {
                self.added.push(a);
            }
        }
        for r in other.removed {
            if !self.removed.contains(&r) {
                self.removed.push(r);
            }
        }
    }
}

/// Compare dependency manifests between two directory trees (original vs candidate).
pub fn dependency_delta_between_trees(
    original_root: &Path,
    candidate_root: &Path,
) -> DependencyDelta {
    let mut delta = DependencyDelta::default();
    for name in ["Cargo.toml", "package.json", "go.mod"] {
        let o = original_root.join(name);
        let c = candidate_root.join(name);
        if !o.exists() && !c.exists() {
            continue;
        }
        let old_text = fs::read_to_string(&o).unwrap_or_default();
        let new_text = fs::read_to_string(&c).unwrap_or_default();
        if old_text == new_text {
            continue;
        }
        delta.merge(dependency_delta_from_texts(name, &old_text, &new_text));
    }
    delta
}

/// Parse dependency names from Cargo.toml / package.json / go.mod text and diff them.
pub fn dependency_delta_from_texts(filename: &str, old: &str, new: &str) -> DependencyDelta {
    let old_deps = parse_manifest_deps(filename, old);
    let new_deps = parse_manifest_deps(filename, new);
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for d in &new_deps {
        if !old_deps.contains(d) {
            added.push(d.clone());
        }
    }
    for d in &old_deps {
        if !new_deps.contains(d) {
            removed.push(d.clone());
        }
    }
    DependencyDelta { added, removed }
}

fn dependency_delta_from_git(repo_root: &Path, paths: &[String]) -> Result<DependencyDelta> {
    Ok(dependency_delta_vs_git_head(repo_root, paths))
}

/// Compare current working-tree manifests to `HEAD` for named paths (best-effort).
pub fn dependency_delta_vs_git_head(repo_root: &Path, paths: &[String]) -> DependencyDelta {
    let mut delta = DependencyDelta::default();
    for p in paths {
        let base = p.rsplit('/').next().unwrap_or(p);
        if !matches!(base, "Cargo.toml" | "package.json" | "go.mod") {
            continue;
        }
        let new_text = fs::read_to_string(repo_root.join(p)).unwrap_or_default();
        let old_text = git_output(repo_root, &["show", &format!("HEAD:{p}")]).unwrap_or_default();
        delta.merge(dependency_delta_from_texts(base, &old_text, &new_text));
    }
    delta
}

fn parse_manifest_deps(filename: &str, text: &str) -> Vec<String> {
    match filename {
        "Cargo.toml" => parse_cargo_deps(text),
        "package.json" => parse_package_json_deps(text),
        "go.mod" => parse_go_mod_deps(text),
        _ => Vec::new(),
    }
}

fn parse_cargo_deps(text: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            // Runtime deps only: skip [dev-dependencies] / [build-dependencies].
            let is_dev_or_build =
                t.contains("dev-dependencies") || t.contains("build-dependencies");
            in_deps = !is_dev_or_build
                && (t == "[dependencies]"
                    || t.starts_with("[dependencies.")
                    || t == "[workspace.dependencies]"
                    || t.contains(".dependencies]"));
            continue;
        }
        if !in_deps || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = t.split_once('=') {
            let name = key.trim().trim_matches('"');
            if !name.is_empty() && !deps.iter().any(|d| d == name) {
                deps.push(name.to_string());
            }
        }
    }
    deps
}

fn parse_package_json_deps(text: &str) -> Vec<String> {
    let mut deps = Vec::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        for key in ["dependencies", "optionalDependencies"] {
            if let Some(obj) = v.get(key).and_then(|x| x.as_object()) {
                for name in obj.keys() {
                    if !deps.contains(name) {
                        deps.push(name.clone());
                    }
                }
            }
        }
        // Intentionally skip devDependencies for runtime dependency counting.
    }
    deps
}

fn parse_go_mod_deps(text: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_require = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("require (") {
            in_require = true;
            continue;
        }
        if in_require {
            if t == ")" {
                in_require = false;
                continue;
            }
            if t.is_empty() || t.starts_with("//") {
                continue;
            }
            let name = t.split_whitespace().next().unwrap_or("");
            if !name.is_empty() && !deps.iter().any(|d| d == name) {
                deps.push(name.to_string());
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("require ") {
            let name = rest.split_whitespace().next().unwrap_or("");
            if !name.is_empty() && !deps.iter().any(|d| d == name) {
                deps.push(name.to_string());
            }
        }
    }
    deps
}

/// Apply generated-code path heuristics to metrics (line counts for matching paths).
pub fn apply_generated_code_heuristics(metrics: &mut DiffMetrics, repo_root: Option<&Path>) {
    let mut gen_lines = 0u32;
    for path in &metrics.changed_paths {
        if !path_looks_generated(path) {
            continue;
        }
        if let Some(root) = repo_root {
            let full = root.join(path);
            if let Ok(text) = fs::read_to_string(&full) {
                gen_lines = gen_lines.saturating_add(text.lines().count() as u32);
                continue;
            }
        }
        // Without readable file content, count at least one unit so the path costs something.
        gen_lines = gen_lines.saturating_add(1);
    }
    if gen_lines > 0 {
        metrics.generated_code_lines = metrics.generated_code_lines.max(gen_lines);
    }
}

/// Whether a path looks like generated / vendored machine output.
pub fn path_looks_generated(path: &str) -> bool {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    let base = p.rsplit('/').next().unwrap_or(&p);
    p.contains("/generated/")
        || p.contains("/gen/")
        || p.contains("/.generated/")
        || p.contains("/__generated__/")
        || p.contains("/node_modules/")
        || p.contains("/vendor/")
        || p.contains("/target/")
        || base.ends_with(".pb.go")
        || base.ends_with(".pb.rs")
        || base.ends_with("_pb2.py")
        || base.ends_with(".min.js")
        || base.ends_with(".min.css")
        || base.ends_with(".map")
        || base.ends_with(".snap")
        || base.ends_with(".lock")
        || base == "cargo.lock"
        || base == "package-lock.json"
        || base == "pnpm-lock.yaml"
        || base == "yarn.lock"
        || base.ends_with("_generated.rs")
        || base.ends_with("_generated.go")
        || base.ends_with(".g.dart")
        || p.contains("/openapi/generated")
        || (base.starts_with("generated_") && (base.ends_with(".rs") || base.ends_with(".go")))
}

/// Public alias used by assessment / policy helpers.
pub fn path_looks_like_test(path: &str) -> bool {
    is_test_path(path)
}

fn status_looks_empty_ok(raw: &str) -> bool {
    raw.is_empty()
}

/// Parse `git status -z` (porcelain v1) records.
///
/// Wire format (verified against git):
/// - Normal: `XY PATH\0` where XY is two status bytes followed by a single space,
///   then the path until NUL.
/// - Rename/copy: `XY NEW\0OLD\0` — **first path is the destination/new path**,
///   second is the source/old path. We keep the new path for `changed_paths`.
///
/// Example rename bytes: `R  new.txt\0old.txt\0`.
fn parse_status_z(raw: &str) -> Vec<(String, String)> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Need at least XY.
        if i + 2 > bytes.len() {
            break;
        }
        let xy = String::from_utf8_lossy(&bytes[i..i + 2]).into_owned();
        i += 2;
        // Porcelain v1 -z: single space after XY (not a second NUL).
        if i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        // path1 until NUL
        let start = i;
        while i < bytes.len() && bytes[i] != 0 {
            i += 1;
        }
        let path1 = String::from_utf8_lossy(&bytes[start..i]).replace('\\', "/");
        if i < bytes.len() && bytes[i] == 0 {
            i += 1;
        } else if i >= bytes.len() && path1.is_empty() {
            break;
        }
        // Rename/copy: second path (old) follows; keep NEW (path1).
        let is_rename_or_copy = xy.chars().next().is_some_and(|c| c == 'R' || c == 'C')
            || xy.chars().nth(1).is_some_and(|c| c == 'R' || c == 'C');
        if is_rename_or_copy && i < bytes.len() {
            // Consume old path until NUL (required field for R/C entries).
            while i < bytes.len() && bytes[i] != 0 {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == 0 {
                i += 1;
            }
        }
        // Destination/new path for renames; sole path otherwise.
        if !path1.is_empty() {
            out.push((xy, path1));
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
    apply_generated_code_heuristics(&mut metrics, Some(root));
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
    // Structured dep delta when manifests differ.
    let dep = dependency_delta_between_trees(original_root, candidate_root);
    if dep.added_count() > 0 {
        metrics.runtime_dependencies_added = dep.added_count();
    }
    apply_generated_code_heuristics(&mut metrics, Some(candidate_root));
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
    // Symlink safety: never follow symlink directories; skip symlink files.
    let cur_meta = fs::symlink_metadata(current)?;
    if cur_meta.file_type().is_symlink() {
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
        // Reject `.` / `..` name components (should not appear from read_dir, but belt+suspenders).
        if name_str == "." || name_str == ".." {
            continue;
        }
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            // Never follow or count symlinks.
            continue;
        }
        if ft.is_dir() {
            if out.len() >= MAX_PATH_MAP_ENTRIES {
                return Err(TifError::Other(format!(
                    "tree walk exceeded {MAX_PATH_MAP_ENTRIES} path entries under {}",
                    root.display()
                )));
            }
            walk_tree(root, &path, out)?;
        } else if ft.is_file() {
            if out.len() >= MAX_PATH_MAP_ENTRIES {
                return Err(TifError::Other(format!(
                    "tree walk exceeded {MAX_PATH_MAP_ENTRIES} path entries under {}",
                    root.display()
                )));
            }
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
    // Bound map growth for pathological inputs.
    let old_lines: Vec<&str> = old.lines().take(MAX_PATH_MAP_ENTRIES).collect();
    let new_lines: Vec<&str> = new.lines().take(MAX_PATH_MAP_ENTRIES).collect();
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
///
/// Returns an error-shaped empty metrics only via panic-free path; oversized
/// input is rejected by [`metrics_from_unified_diff_checked`].
pub fn metrics_from_unified_diff(diff: &str) -> DiffMetrics {
    metrics_from_unified_diff_checked(diff).unwrap_or_default()
}

/// Parse a unified diff, rejecting inputs larger than [`MAX_UNIFIED_DIFF_BYTES`].
pub fn metrics_from_unified_diff_checked(diff: &str) -> Result<DiffMetrics> {
    if diff.len() > MAX_UNIFIED_DIFF_BYTES {
        return Err(TifError::Other(format!(
            "unified diff exceeds size limit ({} bytes > {} bytes / 64 MiB)",
            diff.len(),
            MAX_UNIFIED_DIFF_BYTES
        )));
    }
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

    Ok(metrics)
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
    if path_looks_generated(&norm) {
        // Unified-diff path: at least one generated unit; line counts already in metrics.
        metrics.generated_code_lines = metrics.generated_code_lines.saturating_add(1);
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
    fn parse_status_z_basic_and_untracked() {
        // XY + space + path + NUL
        let raw = " M src/lib.rs\0?? new.txt\0";
        let entries = parse_status_z(raw);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, " M");
        assert_eq!(entries[0].1, "src/lib.rs");
        assert_eq!(entries[1].0, "??");
        assert_eq!(entries[1].1, "new.txt");
    }

    #[test]
    fn parse_status_z_rename_keeps_new_path() {
        // Verified git wire format: `R  new.txt\0old.txt\0` (first path is destination).
        let raw = "R  new.txt\0old.txt\0";
        let entries = parse_status_z(raw);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "R ");
        assert_eq!(
            entries[0].1, "new.txt",
            "must keep NEW/destination path, not old"
        );
    }

    #[test]
    fn parse_status_z_copy_keeps_new_path() {
        let raw = "C  dest/file.rs\0src/file.rs\0";
        let entries = parse_status_z(raw);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "dest/file.rs");
    }

    #[test]
    fn parse_status_z_mixed_rename_and_modify() {
        let raw = "R  b.txt\0a.txt\0 M c.txt\0";
        let entries = parse_status_z(raw);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].1, "b.txt");
        assert_eq!(entries[1].0, " M");
        assert_eq!(entries[1].1, "c.txt");
    }

    #[test]
    fn unified_diff_rejects_oversized_input() {
        // Allocate just over the limit with a tiny header-ish payload.
        let mut huge = String::with_capacity(MAX_UNIFIED_DIFF_BYTES + 8);
        huge.push_str("diff --git a/x b/x\n");
        while huge.len() <= MAX_UNIFIED_DIFF_BYTES {
            huge.push_str("+line\n");
        }
        let err = metrics_from_unified_diff_checked(&huge).unwrap_err();
        assert!(
            err.to_string().contains("size limit") || err.to_string().contains("64 MiB"),
            "{err}"
        );
    }

    #[test]
    fn cargo_dependency_delta_detects_added() {
        let old = r#"
[package]
name = "x"
version = "0.1.0"

[dependencies]
serde = "1"
"#;
        let new = r#"
[package]
name = "x"
version = "0.1.0"

[dependencies]
serde = "1"
tokio = "1"
"#;
        let d = dependency_delta_from_texts("Cargo.toml", old, new);
        assert!(d.added.iter().any(|a| a == "tokio"));
        assert!(!d.added.iter().any(|a| a == "serde"));
        assert!(d.removed.is_empty());
    }

    #[test]
    fn package_json_dependency_delta_skips_dev() {
        let old = r#"{"dependencies":{"a":"1"},"devDependencies":{"jest":"29"}}"#;
        let new =
            r#"{"dependencies":{"a":"1","b":"2"},"devDependencies":{"jest":"29","eslint":"8"}}"#;
        let d = dependency_delta_from_texts("package.json", old, new);
        assert_eq!(d.added, vec!["b".to_string()]);
        assert!(!d.added.iter().any(|x| x == "eslint"));
    }

    #[test]
    fn generated_path_heuristics() {
        assert!(path_looks_generated("src/generated/api.rs"));
        assert!(path_looks_generated("api/v1/types.pb.go"));
        assert!(path_looks_generated("dist/app.min.js"));
        assert!(!path_looks_generated("src/main.rs"));
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
