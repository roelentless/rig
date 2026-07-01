use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{DiscoveredTask, TaskProvider};

/// Discovers tasks from a set of already-resolved, existence-validated Makefiles.
pub struct MakeProvider {
    makefiles: Vec<PathBuf>,
}

impl MakeProvider {
    pub fn new(makefiles: Vec<PathBuf>) -> Self {
        MakeProvider { makefiles }
    }
}

impl TaskProvider for MakeProvider {
    fn name(&self) -> &str {
        "makefile"
    }

    fn discover(&self) -> Vec<DiscoveredTask> {
        let mut result = Vec::new();
        for makefile_path in &self.makefiles {
            let makefile_dir = makefile_path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string());

            // Use `make -f <name>` for non-standard filenames so make can find the file
            let filename = makefile_path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| "Makefile".to_string());
            let make_prefix =
                if filename == "Makefile" || filename == "makefile" || filename == "GNUmakefile" {
                    "make".to_string()
                } else {
                    format!("make -f {}", filename)
                };

            for (target_name, description) in parse_makefile(makefile_path) {
                result.push(DiscoveredTask {
                    command: format!("{} {}", make_prefix, target_name),
                    working_dir: makefile_dir.clone(),
                    description,
                    name: target_name,
                });
            }
        }
        result
    }
}

/// Parse a Makefile and return (target_name, description) for public targets.
///
/// Public targets are those listed in `.PHONY`. If no `.PHONY` is declared,
/// falls back to targets with `## target: description` comments.
/// Descriptions are extracted from `## target: description` comment lines
/// that appear anywhere in the file.
fn parse_makefile(path: &Path) -> Vec<(String, Option<String>)> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Collect .PHONY targets (handles multiple .PHONY declarations)
    let mut phony: HashSet<String> = HashSet::new();
    for line in content.lines() {
        if let Some(rest) = line.trim().strip_prefix(".PHONY:") {
            for target in rest.split_whitespace() {
                phony.insert(target.to_string());
            }
        }
    }

    // Collect ## target: description comments.
    // Only match non-indented lines (## at column 0) to skip continuation comments.
    let mut descriptions: HashMap<String, String> = HashMap::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            // rest must start with the target name directly (no leading whitespace)
            if !rest.starts_with(' ') && !rest.starts_with('\t') {
                if let Some(colon_pos) = rest.find(':') {
                    let target = rest[..colon_pos].trim().to_string();
                    let desc = rest[colon_pos + 1..].trim().to_string();
                    // Valid make target names: no spaces
                    if !target.is_empty() && !target.contains(' ') {
                        descriptions.insert(target, desc);
                    }
                }
            }
        }
    }

    let candidates: Vec<String> = if phony.is_empty() {
        // No .PHONY — expose only targets documented with ## comments
        descriptions.keys().cloned().collect()
    } else {
        // Expose .PHONY targets, skipping internal ones (leading . or _)
        phony
            .into_iter()
            .filter(|t| !t.starts_with('.') && !t.starts_with('_'))
            .collect()
    };

    let mut result: Vec<(String, Option<String>)> = candidates
        .into_iter()
        .map(|name| {
            let desc = descriptions.get(&name).cloned();
            (name, desc)
        })
        .collect();
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}
