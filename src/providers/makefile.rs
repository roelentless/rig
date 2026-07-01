use std::collections::HashSet;
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

/// Parse a Makefile and return `(target_name, description)` for every real target.
///
/// Mirrors `makex` (`lib/parse-targets.awk` + `emit_file` in `bin/makex`):
/// literal `include`/`-include`/`sinclude` directives are resolved and their
/// contents concatenated (parent first, then each include, in file order), then
/// every target that isn't a recipe line, comment, assignment, pattern rule,
/// special (`.`-prefixed), variable-expanded (`$`), or malformed name is emitted
/// in first-seen order. Inline `## doc` after a target rule becomes the
/// description; an empty or absent doc becomes `None`. Dedup is first-wins.
fn parse_makefile(path: &Path) -> Vec<(String, Option<String>)> {
    let mut content = String::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    emit_file(path, &mut content, &mut visited);
    parse_targets(&content)
}

/// Concatenate `path` and its literal includes into `out`, in file order.
/// Dedups by resolved absolute path to avoid include loops.
fn emit_file(path: &Path, out: &mut String, visited: &mut HashSet<PathBuf>) {
    let key = abs_key(path);
    if !visited.insert(key) {
        return;
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    out.push_str(&content);
    if !content.ends_with('\n') {
        out.push('\n');
    }

    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    for line in content.lines() {
        if let Some(tokens) = parse_include_line(line) {
            for token in tokens {
                // Skip unresolved variable references.
                if token.contains('$') {
                    continue;
                }
                let inc = dir.join(&token);
                if inc.is_file() {
                    emit_file(&inc, out, visited);
                }
            }
        }
    }
}

/// Resolve a path to a stable dedup key: canonicalized parent dir + filename.
/// Mirrors makex's `cd "$(dirname f)" && pwd)/$(basename f)` (resolves the
/// directory, not the file itself). Falls back to the raw path.
fn abs_key(path: &Path) -> PathBuf {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    match (parent.canonicalize(), path.file_name()) {
        (Ok(dir), Some(name)) => dir.join(name),
        _ => path.to_path_buf(),
    }
}

/// If `line` is an include directive, return its file tokens (after stripping a
/// trailing `# comment`). Matches `^\s*-?(include|sinclude)\s+(.+)$`.
fn parse_include_line(line: &str) -> Option<Vec<String>> {
    let t = line.trim_start();
    let t = t.strip_prefix('-').unwrap_or(t);
    let rest = t
        .strip_prefix("sinclude")
        .or_else(|| t.strip_prefix("include"))?;
    // Require at least one whitespace char after the keyword.
    if !rest.starts_with(|c: char| c == ' ' || c == '\t') {
        return None;
    }
    // Strip a trailing `# comment`, then split on whitespace.
    let rest = rest.split('#').next().unwrap_or("");
    let tokens: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
    if tokens.is_empty() {
        return None;
    }
    Some(tokens)
}

/// Extract `(target, doc)` pairs from concatenated Makefile content, applying the
/// awk's exact filtering and first-wins dedup, preserving first-seen order.
fn parse_targets(content: &str) -> Vec<(String, Option<String>)> {
    let mut result: Vec<(String, Option<String>)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for raw in content.lines() {
        // Recipe line (literal TAB) — skip so we don't match shell colons.
        if raw.starts_with('\t') {
            continue;
        }
        // Comment-only line.
        if raw.trim_start().starts_with('#') {
            continue;
        }

        // Strip trailing CR (CRLF files).
        let line = raw.strip_suffix('\r').unwrap_or(raw);

        // First colon.
        let ci = match line.find(':') {
            Some(i) => i,
            None => continue,
        };

        // Variable assignment: `:=`, `::=` handled by awk as-is (only the char
        // immediately after the first colon is checked for `=`).
        if line[ci + 1..].starts_with('=') {
            continue;
        }

        let targets = line[..ci].trim();
        if targets.is_empty() {
            continue;
        }
        // `=` anywhere in the LHS (e.g. `export FOO = bar`) → assignment.
        if targets.contains('=') {
            continue;
        }

        // Optional `## doc` from anywhere on the line. Empty doc collapses to None.
        let doc = line
            .find("##")
            .map(|hi| line[hi + 2..].trim())
            .filter(|d| !d.is_empty())
            .map(|d| d.to_string());

        for t in targets.split_whitespace() {
            if t.contains('%') {
                continue; // pattern rule
            }
            if t.starts_with('.') {
                continue; // special target (.PHONY, .DEFAULT, …)
            }
            if t.contains('$') {
                continue; // variable-expanded target
            }
            if !valid_target_name(t) {
                continue;
            }
            if !seen.insert(t.to_string()) {
                continue; // first occurrence wins
            }
            result.push((t.to_string(), doc.clone()));
        }
    }

    result
}

/// Matches the awk name filter `^[A-Za-z0-9_+/-][A-Za-z0-9_.+/-]*$`.
fn valid_target_name(t: &str) -> bool {
    let is_first = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '/' | '-');
    let is_rest = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '/' | '-');
    let mut chars = t.chars();
    match chars.next() {
        Some(first) if is_first(first) => chars.all(is_rest),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Write `content` to `<dir>/<name>` and return the full path.
    fn write(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn names(pairs: &[(String, Option<String>)]) -> Vec<String> {
        pairs.iter().map(|(n, _)| n.clone()).collect()
    }

    #[test]
    fn simple_all_real_targets_in_file_order() {
        // Mirrors makex fixtures/simple: assignments, .PHONY, and pattern rule
        // are all filtered; real targets surface in file order.
        let dir = TempDir::new().unwrap();
        let mf = write(
            &dir,
            "Makefile",
            "\
CC := gcc
CFLAGS = -O2

.PHONY: build test clean

build:
\t@echo building

test: build
\t@echo testing

clean:
\t@rm -f *.o

%.o: %.c
\t$(CC) $(CFLAGS) -c $<
",
        );

        let out = parse_makefile(&mf);
        assert_eq!(names(&out), vec!["build", "test", "clean"]);
        // No docs in this fixture.
        assert!(out.iter().all(|(_, d)| d.is_none()));
    }

    #[test]
    fn documented_inline_docs_and_undocumented_target() {
        // Mirrors makex fixtures/documented.
        let dir = TempDir::new().unwrap();
        let mf = write(
            &dir,
            "Makefile",
            "\
.DEFAULT_GOAL := test

.PHONY: build test deploy

build: ## Compile the binary
\t@echo build

test: build ## Run the test suite
\t@echo test

deploy: ## Ship to production
\t@echo deploy

internal-helper:
\t@echo no doc here
",
        );

        let out = parse_makefile(&mf);
        assert_eq!(
            out,
            vec![
                ("build".into(), Some("Compile the binary".into())),
                ("test".into(), Some("Run the test suite".into())),
                ("deploy".into(), Some("Ship to production".into())),
                ("internal-helper".into(), None),
            ]
        );
    }

    #[test]
    fn phony_declared_targets_still_work() {
        // A .PHONY declaration no longer gates discovery — the targets it names
        // surface because they are real rules, and .PHONY itself is filtered.
        let dir = TempDir::new().unwrap();
        let mf = write(
            &dir,
            "Makefile",
            "\
.PHONY: build test clean

## build: compile the project
build:
\t@echo build

test:
\t@echo test

clean:
\t@echo clean
",
        );

        let out = parse_makefile(&mf);
        assert_eq!(names(&out), vec!["build", "test", "clean"]);
        // `## build:` here is a comment-only line (no rule target before `##`
        // beyond the target itself); the doc lives on the `build:` line only if
        // inline. This comment form yields no inline doc.
        assert!(out.iter().all(|(_, d)| d.is_none()));
    }

    #[test]
    fn excludes_assignments_patterns_special_and_var_targets() {
        let dir = TempDir::new().unwrap();
        let mf = write(
            &dir,
            "Makefile",
            "\
VAR := x
export FOO = bar
OTHER = y

.PHONY: real
.DEFAULT: whatever

%.o: %.c
\t@echo pattern

$(GEN): dep
\t@echo generated

real:
\t@echo real
",
        );

        let out = parse_makefile(&mf);
        assert_eq!(names(&out), vec!["real"]);
    }

    #[test]
    fn includes_targets_from_both_files_in_order() {
        // Mirrors makex fixtures/includes: parent content first, then the
        // included file's content.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "extra.mk",
            "\
.PHONY: from-include

from-include: ## Defined in extra.mk
\t@echo included
",
        );
        let mf = write(
            &dir,
            "Makefile",
            "\
include extra.mk

.PHONY: top

top: ## Top-level target
\t@echo top
",
        );

        let out = parse_makefile(&mf);
        assert_eq!(
            out,
            vec![
                ("top".into(), Some("Top-level target".into())),
                ("from-include".into(), Some("Defined in extra.mk".into())),
            ]
        );
    }

    #[test]
    fn dedup_first_occurrence_wins() {
        let dir = TempDir::new().unwrap();
        let mf = write(
            &dir,
            "Makefile",
            "\
build: ## first doc
\t@echo one

build: ## second doc
\t@echo two
",
        );

        let out = parse_makefile(&mf);
        assert_eq!(out, vec![("build".into(), Some("first doc".into()))]);
    }

    #[test]
    fn optional_include_missing_file_is_skipped() {
        // `-include` of a nonexistent file must not error; parent targets stand.
        let dir = TempDir::new().unwrap();
        let mf = write(
            &dir,
            "Makefile",
            "\
-include does-not-exist.mk

build:
\t@echo build
",
        );

        let out = parse_makefile(&mf);
        assert_eq!(names(&out), vec!["build"]);
    }

    #[test]
    fn discover_preserves_file_order_and_docs() {
        let dir = TempDir::new().unwrap();
        let mf = write(
            &dir,
            "Makefile",
            "\
alpha: ## first
\t@echo a

beta:
\t@echo b
",
        );

        let provider = MakeProvider::new(vec![mf]);
        let tasks = provider.discover();
        assert_eq!(
            tasks.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        assert_eq!(tasks[0].description.as_deref(), Some("first"));
        assert_eq!(tasks[1].description, None);
    }
}
