use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::output::log_verbose;

// ============================================================================
// ERRORS
// ============================================================================

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("{0}")]
    Generic(String),
}

impl ConfigError {
    pub fn generic(msg: impl Into<String>) -> Self {
        ConfigError::Generic(msg.into())
    }
}

// ============================================================================
// TYPES
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthCheck {
    pub grace_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EnvFileSpec {
    Single(String),
    Multiple(Vec<EnvFileItem>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EnvFileItem {
    Path(String),
    Entry(EnvFileEntry),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvFileEntry {
    pub path: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatchDef {
    pub paths: Option<Vec<String>>,
    pub extensions: Option<Vec<String>>,
    pub patterns: Option<Vec<String>>,
    pub ignore: Option<Vec<String>>,
    pub debounce: Option<String>,
}

/// Where a task originated. Rig-authored tasks win over Makefile targets on a
/// short-name clash (force-over-auto precedence); the tag carries that identity
/// through the unified group tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TaskSource {
    #[default]
    Rig,
    Make,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDef {
    pub command: String,
    pub working_dir: Option<String>,
    /// INLINE `environment:` only. Env files are recorded (not loaded) in
    /// `env_files` and materialized at run time.
    pub environment: Option<HashMap<String, String>>,
    pub env_file: Option<EnvFileSpec>,
    /// Resolved env-file entries (paths + `required`), NOT loaded at parse time.
    #[serde(skip)]
    pub env_files: Vec<ResolvedEnvFileEntry>,
    pub description: Option<String>,
    /// Rig-authored (default) vs discovered Makefile target. Not serialized.
    #[serde(skip)]
    pub source: TaskSource,
    /// True when this is its Makefile's default goal (MAKE tasks only). Marks
    /// the `→` row in listings. Not serialized.
    #[serde(skip)]
    pub default_goal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequirementDef {
    pub check: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDef {
    pub command: String,
    pub working_dir: String,
    /// INLINE `environment:` only. Env files are recorded (not loaded) in
    /// `env_files` and materialized at start time.
    pub environment: Option<HashMap<String, String>>,
    pub env_file: Option<EnvFileSpec>,
    /// Resolved env-file entries (paths + `required`), NOT loaded at parse time.
    #[serde(skip)]
    pub env_files: Vec<ResolvedEnvFileEntry>,
    pub color: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub healthcheck: Option<HealthCheck>,
    pub tasks: Option<HashMap<String, TaskDef>>,
    pub watch: Option<WatchDef>,
    pub requirements: Option<Vec<RequirementDef>>,
}

// ============================================================================
// GROUP TREE
// ============================================================================

/// Properties that cascade down the group tree, ancestor-wins.
#[derive(Debug, Clone, Default)]
pub struct Props {
    /// Nearest-explicit working dir for units that omit their own (absolute).
    pub working_dir: Option<String>,
    /// INLINE `environment:` for this level only. Env files are NOT loaded here;
    /// they are recorded in `env_files` and materialized at run/start time.
    pub env: HashMap<String, String>,
    /// Resolved env-file entries (paths + `required`) for this level, NOT loaded.
    pub env_files: Vec<ResolvedEnvFileEntry>,
}

/// One level of the config hierarchy. Backed by a discovered directory (`dir`),
/// explicit rig files (`paths`), inline units, and/or child groups. `name` is
/// `""` at the root (CWD → bare names); a folder name or authored group name
/// otherwise.
#[derive(Debug, Clone)]
pub struct Group {
    pub name: String,
    pub dir: Option<PathBuf>,
    pub paths: Option<Vec<PathBuf>>,
    pub props: Props,
    pub tasks: Vec<(String, TaskDef)>,
    pub services: Vec<(String, ServiceDef)>,
    pub groups: Vec<Group>,
}

impl Group {
    fn empty(name: &str, dir: Option<PathBuf>) -> Self {
        Group {
            name: name.to_string(),
            dir,
            paths: None,
            props: Props::default(),
            tasks: Vec::new(),
            services: Vec::new(),
            groups: Vec::new(),
        }
    }
}

// ============================================================================
// RESOLVED TYPES
// ============================================================================

#[derive(Debug, Clone)]
pub struct ResolvedService {
    pub group: String,
    pub name: String,
    pub def: ServiceDef,
}

#[derive(Debug, Clone)]
pub struct ResolvedTask {
    pub path: String,
    pub group: String,
    pub service: Option<String>,
    pub name: String,
    pub command: String,
    pub working_dir: String,
    pub environment: Option<HashMap<String, String>>,
    pub description: Option<String>,
    pub source: TaskSource,
    /// This task is its Makefile's default goal (MAKE tasks only; rig never).
    pub default_goal: bool,
}

// ============================================================================
// CONSTANTS
// ============================================================================

pub const CONFIG_NAMES: &[&str] = &["rig.yaml", "rig.yml"];
pub const LOG_DIR: &str = ".rig/logs";

fn is_rig_file_name(name: &str) -> bool {
    name == "rig.yaml" || name == "rig.yml" || name.ends_with(".rig.yaml")
}

// ============================================================================
// SCHEMA VALIDATION
// ============================================================================

fn root_keys() -> HashSet<&'static str> {
    [
        "tasks",
        "services",
        "environment",
        "env_file",
        "working_dir",
        "groups",
    ]
    .into_iter()
    .collect()
}
fn group_keys() -> HashSet<&'static str> {
    [
        "dir",
        "paths",
        "tasks",
        "services",
        "environment",
        "env_file",
        "working_dir",
        "groups",
    ]
    .into_iter()
    .collect()
}
fn service_keys() -> HashSet<&'static str> {
    [
        "command",
        "working_dir",
        "environment",
        "env_file",
        "color",
        "depends_on",
        "healthcheck",
        "tasks",
        "watch",
        "requirements",
    ]
    .into_iter()
    .collect()
}
fn task_keys() -> HashSet<&'static str> {
    [
        "command",
        "working_dir",
        "environment",
        "env_file",
        "description",
    ]
    .into_iter()
    .collect()
}
fn watch_keys() -> HashSet<&'static str> {
    ["paths", "extensions", "patterns", "ignore", "debounce"]
        .into_iter()
        .collect()
}
fn healthcheck_keys() -> HashSet<&'static str> {
    ["grace_ms"].into_iter().collect()
}
fn env_file_entry_keys() -> HashSet<&'static str> {
    ["path", "required"].into_iter().collect()
}
fn requirement_keys() -> HashSet<&'static str> {
    ["check", "command"].into_iter().collect()
}

fn validate_keys(
    obj: &serde_yaml::Mapping,
    allowed: &HashSet<&str>,
    context: &str,
    config_path: &str,
    errors: &mut Vec<String>,
) {
    for key in obj.keys() {
        if let Some(key_str) = key.as_str() {
            if !allowed.contains(key_str) {
                let mut valid: Vec<&&str> = allowed.iter().collect();
                valid.sort();
                let valid_str: Vec<&str> = valid.into_iter().copied().collect();
                errors.push(format!(
                    "Unknown key '{}' in {} ({}). Valid keys: {}",
                    key_str,
                    context,
                    config_path,
                    valid_str.join(", ")
                ));
            }
        }
    }
}

fn validate_service(
    svc_label: &str,
    s: &serde_yaml::Mapping,
    config_path: &str,
    errors: &mut Vec<String>,
) {
    validate_keys(
        s,
        &service_keys(),
        &format!("service '{}'", svc_label),
        config_path,
        errors,
    );

    if let Some(w) = s.get("watch").and_then(|v| v.as_mapping()) {
        validate_keys(
            w,
            &watch_keys(),
            &format!("watch in service '{}'", svc_label),
            config_path,
            errors,
        );
    }
    if let Some(hc) = s.get("healthcheck").and_then(|v| v.as_mapping()) {
        validate_keys(
            hc,
            &healthcheck_keys(),
            &format!("healthcheck in service '{}'", svc_label),
            config_path,
            errors,
        );
    }
    if let Some(arr) = s.get("env_file").and_then(|v| v.as_sequence()) {
        for entry in arr {
            if let Some(e) = entry.as_mapping() {
                validate_keys(
                    e,
                    &env_file_entry_keys(),
                    &format!("env_file entry in service '{}'", svc_label),
                    config_path,
                    errors,
                );
            }
        }
    }
    if let Some(arr) = s.get("requirements").and_then(|v| v.as_sequence()) {
        for entry in arr {
            if let Some(r) = entry.as_mapping() {
                validate_keys(
                    r,
                    &requirement_keys(),
                    &format!("requirement in service '{}'", svc_label),
                    config_path,
                    errors,
                );
            }
        }
    }
    if let Some(tasks) = s.get("tasks").and_then(|v| v.as_mapping()) {
        for (tk, tv) in tasks {
            let tname = tk.as_str().unwrap_or("?");
            if let Some(t) = tv.as_mapping() {
                validate_keys(
                    t,
                    &task_keys(),
                    &format!("task '{}.{}'", svc_label, tname),
                    config_path,
                    errors,
                );
            }
        }
    }
}

/// Validate the tasks/services blocks of one level (root or a group body).
fn validate_units(
    level: &serde_yaml::Mapping,
    label: &str,
    config_path: &str,
    errors: &mut Vec<String>,
) {
    if let Some(services) = level.get("services").and_then(|v| v.as_mapping()) {
        for (svc_key, svc_val) in services {
            let svc_name = svc_key.as_str().unwrap_or("?");
            if let Some(s) = svc_val.as_mapping() {
                let svc_label = if label.is_empty() {
                    svc_name.to_string()
                } else {
                    format!("{}.{}", label, svc_name)
                };
                validate_service(&svc_label, s, config_path, errors);
            }
        }
    }
    if let Some(tasks) = level.get("tasks").and_then(|v| v.as_mapping()) {
        for (tk, tv) in tasks {
            let tname = tk.as_str().unwrap_or("?");
            if let Some(t) = tv.as_mapping() {
                let tlabel = if label.is_empty() {
                    tname.to_string()
                } else {
                    format!("{}.{}", label, tname)
                };
                validate_keys(
                    t,
                    &task_keys(),
                    &format!("task '{}'", tlabel),
                    config_path,
                    errors,
                );
            }
        }
    }
}

fn validate_groups(groups: &serde_yaml::Mapping, config_path: &str, errors: &mut Vec<String>) {
    for (group_key, group_val) in groups {
        let group_name = group_key.as_str().unwrap_or("?");
        if let Some(g) = group_val.as_mapping() {
            validate_keys(
                g,
                &group_keys(),
                &format!("group '{}'", group_name),
                config_path,
                errors,
            );
            validate_units(g, group_name, config_path, errors);
            if let Some(children) = g.get("groups").and_then(|v| v.as_mapping()) {
                validate_groups(children, config_path, errors);
            }
        }
    }
}

fn validate_config_schema(raw: &serde_yaml::Value, config_path: &str) -> Result<(), ConfigError> {
    let mut errors = Vec::new();

    if let Some(root) = raw.as_mapping() {
        validate_keys(root, &root_keys(), "config root", config_path, &mut errors);
        validate_units(root, "", config_path, &mut errors);
        if let Some(groups) = root.get("groups").and_then(|v| v.as_mapping()) {
            validate_groups(groups, config_path, &mut errors);
        }
    }

    if !errors.is_empty() {
        return Err(ConfigError::generic(format!(
            "Config validation failed:\n  {}",
            errors.join("\n  ")
        )));
    }
    Ok(())
}

// ============================================================================
// PATH & ENV HELPERS
// ============================================================================

fn normalize_path(path: &str) -> String {
    let p = PathBuf::from(path);
    let mut components = Vec::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            _ => components.push(comp),
        }
    }
    let result: PathBuf = components.into_iter().collect();
    result.to_string_lossy().to_string()
}

fn resolve_path(path: &str, base_dir: &str) -> String {
    if path.starts_with('/') {
        normalize_path(path)
    } else {
        normalize_path(&format!("{}/{}", base_dir, path))
    }
}

fn normalize_env_values(env: &HashMap<String, serde_yaml::Value>) -> HashMap<String, String> {
    env.iter()
        .map(|(k, v)| {
            let val = match v {
                serde_yaml::Value::String(s) => s.clone(),
                serde_yaml::Value::Number(n) => n.to_string(),
                serde_yaml::Value::Bool(b) => b.to_string(),
                _ => format!("{:?}", v),
            };
            (k.clone(), val)
        })
        .collect()
}

fn load_env_files(
    entries: &[ResolvedEnvFileEntry],
) -> Result<HashMap<String, String>, ConfigError> {
    let mut result = HashMap::new();

    for entry in entries {
        match std::fs::read_to_string(&entry.path) {
            Ok(content) => {
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    if let Some(eq_pos) = line.find('=') {
                        let key = line[..eq_pos].trim().to_string();
                        let mut val = line[eq_pos + 1..].trim().to_string();
                        if (val.starts_with('"') && val.ends_with('"'))
                            || (val.starts_with('\'') && val.ends_with('\''))
                        {
                            val = val[1..val.len() - 1].to_string();
                        }
                        result.insert(key, val);
                    }
                }
            }
            Err(e) => {
                if entry.required {
                    return Err(ConfigError::generic(format!(
                        "Failed to load env file '{}': {}",
                        entry.original_path, e
                    )));
                }
            }
        }
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct ResolvedEnvFileEntry {
    path: String,
    original_path: String,
    required: bool,
}

fn resolve_env_file_spec(spec: &serde_yaml::Value, config_dir: &str) -> Vec<ResolvedEnvFileEntry> {
    let mut entries = Vec::new();

    match spec {
        serde_yaml::Value::String(s) => {
            let resolved = resolve_path(s, config_dir);
            entries.push(ResolvedEnvFileEntry {
                path: resolved,
                original_path: s.clone(),
                required: true,
            });
        }
        serde_yaml::Value::Sequence(arr) => {
            for item in arr {
                match item {
                    serde_yaml::Value::String(s) => {
                        let resolved = resolve_path(s, config_dir);
                        entries.push(ResolvedEnvFileEntry {
                            path: resolved,
                            original_path: s.clone(),
                            required: true,
                        });
                    }
                    serde_yaml::Value::Mapping(m) => {
                        if let Some(path_val) = m.get("path") {
                            let path_str = path_val.as_str().unwrap_or("");
                            let resolved = resolve_path(path_str, config_dir);
                            let required =
                                m.get("required").and_then(|v| v.as_bool()).unwrap_or(true);
                            entries.push(ResolvedEnvFileEntry {
                                path: resolved,
                                original_path: path_str.to_string(),
                                required,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    entries
}

/// Parse an inline `environment:` mapping into normalized string pairs.
fn parse_inline_env(m: &serde_yaml::Mapping) -> Option<HashMap<String, String>> {
    m.get("environment")
        .and_then(|v| v.as_mapping())
        .map(|env_map| {
            let map: HashMap<String, serde_yaml::Value> = env_map
                .iter()
                .filter_map(|(k, v)| k.as_str().map(|ks| (ks.to_string(), v.clone())))
                .collect();
            normalize_env_values(&map)
        })
}

/// Split a level's env declaration into (inline map, resolved env-file entries)
/// WITHOUT touching the filesystem. Files are loaded later at run/start time by
/// `fold_level_env`. This keeps listing/discovery env-file-free.
fn split_level_env(
    m: &serde_yaml::Mapping,
    config_dir: &str,
) -> (HashMap<String, String>, Vec<ResolvedEnvFileEntry>) {
    let inline = parse_inline_env(m).unwrap_or_default();
    let entries = m
        .get("env_file")
        .map(|ef| resolve_env_file_spec(ef, config_dir))
        .unwrap_or_default();
    (inline, entries)
}

/// Materialize one level's effective env: load its env files (base), then apply
/// its inline env on top (inline overrides the file). Fail fast on a
/// required-missing file. Called only on the run/start path, never on listing.
fn fold_level_env(
    entries: &[ResolvedEnvFileEntry],
    inline: &HashMap<String, String>,
) -> Result<HashMap<String, String>, ConfigError> {
    let mut env = if entries.is_empty() {
        HashMap::new()
    } else {
        load_env_files(entries)?
    };
    for (k, v) in inline {
        env.insert(k.clone(), v.clone());
    }
    Ok(env)
}

// ============================================================================
// UNIT PARSING (service / task) — reused verbatim across every level
// ============================================================================

fn parse_watch(s: &serde_yaml::Mapping, working_dir: &str) -> Option<WatchDef> {
    let w = s.get("watch").and_then(|v| v.as_mapping())?;
    let mut wd = WatchDef::default();
    if let Some(seq) = w.get("paths").and_then(|v| v.as_sequence()) {
        wd.paths = Some(
            seq.iter()
                .filter_map(|p| p.as_str().map(|s| resolve_path(s, working_dir)))
                .collect(),
        );
    }
    if let Some(seq) = w.get("extensions").and_then(|v| v.as_sequence()) {
        wd.extensions = Some(
            seq.iter()
                .filter_map(|e| e.as_str().map(String::from))
                .collect(),
        );
    }
    if let Some(seq) = w.get("patterns").and_then(|v| v.as_sequence()) {
        wd.patterns = Some(
            seq.iter()
                .filter_map(|p| p.as_str().map(String::from))
                .collect(),
        );
    }
    if let Some(seq) = w.get("ignore").and_then(|v| v.as_sequence()) {
        wd.ignore = Some(
            seq.iter()
                .filter_map(|i| i.as_str().map(String::from))
                .collect(),
        );
    }
    if let Some(deb_val) = w.get("debounce") {
        wd.debounce = Some(
            deb_val
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("{}", deb_val.as_u64().unwrap_or(0))),
        );
    }
    Some(wd)
}

/// Parse a task belonging to a service (working_dir stays optional; falls back
/// to the service dir at flatten time).
fn parse_service_task(
    label: &str,
    t: &serde_yaml::Mapping,
    config_dir: &str,
    config_path: &str,
) -> Result<TaskDef, ConfigError> {
    let command = t
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ConfigError::generic(format!(
                "Task '{}' in {} must have a 'command' field",
                label, config_path
            ))
        })?
        .to_string();

    let working_dir = t
        .get("working_dir")
        .and_then(|v| v.as_str())
        .map(|wd| resolve_path(wd, config_dir));

    let (inline, env_files) = split_level_env(t, config_dir);
    let environment = if inline.is_empty() {
        None
    } else {
        Some(inline)
    };

    Ok(TaskDef {
        command,
        working_dir,
        environment,
        env_file: None,
        env_files,
        description: t
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        source: TaskSource::Rig,
        default_goal: false,
    })
}

fn parse_service(
    label: &str,
    s: &serde_yaml::Mapping,
    config_dir: &str,
    config_path: &str,
) -> Result<ServiceDef, ConfigError> {
    let command = s
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ConfigError::generic(format!(
                "Service '{}' in {} must have a 'command' field",
                label, config_path
            ))
        })?
        .to_string();

    let working_dir_raw = s
        .get("working_dir")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ConfigError::generic(format!(
                "Service '{}' in {} must have a 'working_dir' field",
                label, config_path
            ))
        })?;
    let working_dir = resolve_path(working_dir_raw, config_dir);

    let (inline, env_files) = split_level_env(s, config_dir);
    let environment = if inline.is_empty() {
        None
    } else {
        Some(inline)
    };

    let depends_on = s.get("depends_on").and_then(|v| {
        v.as_sequence().map(|seq| {
            seq.iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect::<Vec<String>>()
        })
    });

    let healthcheck = s.get("healthcheck").and_then(|v| {
        v.as_mapping().map(|m| HealthCheck {
            grace_ms: m.get("grace_ms").and_then(|v| v.as_u64()),
        })
    });

    let color = s.get("color").and_then(|v| v.as_str()).map(String::from);

    let watch = parse_watch(s, &working_dir);

    let requirements = if let Some(seq) = s.get("requirements").and_then(|v| v.as_sequence()) {
        let mut reqs = Vec::new();
        for entry in seq {
            let m = entry.as_mapping().ok_or_else(|| {
                ConfigError::generic(format!(
                    "Invalid requirement entry in service '{}' ({}): must be an object with 'check' and 'command'",
                    label, config_path
                ))
            })?;
            let check = m
                .get("check")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ConfigError::generic(format!(
                        "Requirement in service '{}' ({}) must have a 'check' string",
                        label, config_path
                    ))
                })?
                .to_string();
            let cmd = m
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ConfigError::generic(format!(
                        "Requirement in service '{}' ({}) must have a 'command' string",
                        label, config_path
                    ))
                })?
                .to_string();
            reqs.push(RequirementDef {
                check,
                command: cmd,
            });
        }
        Some(reqs)
    } else {
        None
    };

    let svc_tasks = if let Some(tasks) = s.get("tasks").and_then(|v| v.as_mapping()) {
        let mut tm = HashMap::new();
        for (tk, tv) in tasks {
            let tname = tk.as_str().unwrap_or("?");
            let t = tv.as_mapping().ok_or_else(|| {
                ConfigError::generic(format!(
                    "Task '{}.{}' in {} must be a mapping",
                    label, tname, config_path
                ))
            })?;
            let tdef =
                parse_service_task(&format!("{}.{}", label, tname), t, config_dir, config_path)?;
            tm.insert(tname.to_string(), tdef);
        }
        Some(tm)
    } else {
        None
    };

    Ok(ServiceDef {
        command,
        working_dir,
        environment,
        env_file: None,
        env_files,
        color,
        depends_on,
        healthcheck,
        tasks: svc_tasks,
        watch,
        requirements,
    })
}

/// Parse a group/root-level task. `default_wd` is the nearest-explicit working
/// dir; a group task must resolve to some working dir (its own or inherited).
fn parse_group_task(
    label: &str,
    t: &serde_yaml::Mapping,
    config_dir: &str,
    config_path: &str,
    default_wd: &Option<String>,
) -> Result<TaskDef, ConfigError> {
    let command = t
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ConfigError::generic(format!(
                "Task '{}' in {} must have a 'command' field",
                label, config_path
            ))
        })?
        .to_string();

    let working_dir = match t.get("working_dir").and_then(|v| v.as_str()) {
        Some(wd) => resolve_path(wd, config_dir),
        None => default_wd.clone().ok_or_else(|| {
            ConfigError::generic(format!(
                "Task '{}' in {} must have a 'working_dir' field (no group-level working_dir set)",
                label, config_path
            ))
        })?,
    };

    let (inline, env_files) = split_level_env(t, config_dir);
    let environment = if inline.is_empty() {
        None
    } else {
        Some(inline)
    };

    Ok(TaskDef {
        command,
        working_dir: Some(working_dir),
        environment,
        env_file: None,
        env_files,
        description: t
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        source: TaskSource::Rig,
        default_goal: false,
    })
}

// ============================================================================
// FILE PARSING → one level (props + units + child group bodies)
// ============================================================================

/// A single rig file parsed into one group's worth of props, units, and the
/// raw bodies of any authored child `groups:` (built lazily against this file's
/// own directory, preserving folder-aware relative paths).
struct ParsedFile {
    dir: String,
    props: Props,
    tasks: Vec<(String, TaskDef)>,
    services: Vec<(String, ServiceDef)>,
    child_groups: Vec<(String, serde_yaml::Value)>,
}

fn parse_file(path: &str) -> Result<ParsedFile, ConfigError> {
    let config_dir = Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    let content = std::fs::read_to_string(path).map_err(|e| {
        ConfigError::generic(format!("Failed to read config file '{}': {}", path, e))
    })?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|e| ConfigError::generic(format!("Invalid YAML in {}: {}", path, e)))?;
    validate_config_schema(&yaml, path)?;

    parse_level(&yaml, "", &config_dir, path)
}

/// Parse one level (root file body or an inline group body) into a ParsedFile.
fn parse_level(
    yaml: &serde_yaml::Value,
    label: &str,
    config_dir: &str,
    config_path: &str,
) -> Result<ParsedFile, ConfigError> {
    let map = match yaml.as_mapping() {
        Some(m) => m,
        None => {
            return Ok(ParsedFile {
                dir: config_dir.to_string(),
                props: Props::default(),
                tasks: Vec::new(),
                services: Vec::new(),
                child_groups: Vec::new(),
            })
        }
    };

    let working_dir = map
        .get("working_dir")
        .and_then(|v| v.as_str())
        .map(|wd| resolve_path(wd, config_dir));

    let (env, env_files) = split_level_env(map, config_dir);
    let props = Props {
        working_dir,
        env,
        env_files,
    };

    let mut services = Vec::new();
    if let Some(svcs) = map.get("services").and_then(|v| v.as_mapping()) {
        for (svc_key, svc_val) in svcs {
            let svc_name = svc_key.as_str().unwrap_or("?");
            let s = svc_val.as_mapping().ok_or_else(|| {
                ConfigError::generic(format!(
                    "Service '{}' in {} must be a mapping",
                    svc_name, config_path
                ))
            })?;
            let lbl = if label.is_empty() {
                svc_name.to_string()
            } else {
                format!("{}.{}", label, svc_name)
            };
            let def = parse_service(&lbl, s, config_dir, config_path)?;
            services.push((svc_name.to_string(), def));
        }
    }

    let mut tasks = Vec::new();
    if let Some(tsks) = map.get("tasks").and_then(|v| v.as_mapping()) {
        for (tk, tv) in tsks {
            let tname = tk.as_str().unwrap_or("?");
            let t = tv.as_mapping().ok_or_else(|| {
                ConfigError::generic(format!(
                    "Task '{}' in {} must be a mapping",
                    tname, config_path
                ))
            })?;
            let lbl = if label.is_empty() {
                tname.to_string()
            } else {
                format!("{}.{}", label, tname)
            };
            let def = parse_group_task(&lbl, t, config_dir, config_path, &props.working_dir)?;
            tasks.push((tname.to_string(), def));
        }
    }

    let mut child_groups = Vec::new();
    if let Some(groups) = map.get("groups").and_then(|v| v.as_mapping()) {
        for (gk, gv) in groups {
            let gname = gk.as_str().unwrap_or("?").to_string();
            if !gname
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                return Err(ConfigError::generic(format!(
                    "Invalid group name '{}' in {}: must be alphanumeric with hyphens/underscores only",
                    gname, config_path
                )));
            }
            child_groups.push((gname, gv.clone()));
        }
    }

    Ok(ParsedFile {
        dir: config_dir.to_string(),
        props,
        tasks,
        services,
        child_groups,
    })
}

// ============================================================================
// TREE BUILDING
// ============================================================================

/// Pick the config file directly inside `dir` (rig.yaml > rig.yml > *.rig.yaml).
fn pick_config_in_dir(dir: &str) -> Option<PathBuf> {
    for name in CONFIG_NAMES {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut star: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .map(|n| {
                        let n = n.to_string_lossy();
                        n.ends_with(".rig.yaml") && n != "rig.yaml"
                    })
                    .unwrap_or(false)
        })
        .collect();
    star.sort();
    star.into_iter().next()
}

/// Every rig config file under `dir` (gitignore-aware, hidden dirs skipped).
fn scan_rig_files(dir: &str) -> Vec<PathBuf> {
    crate::commands::walk_ignored_files(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            p.file_name()
                .map(|n| is_rig_file_name(&n.to_string_lossy()))
                .unwrap_or(false)
        })
        .collect()
}

/// Every standard-named Makefile under `dir` (gitignore-aware, hidden dirs
/// skipped), used to derive folder-auto child-group segments. The group's own
/// Makefile is picked separately via `own_makefile`.
fn scan_makefiles(dir: &str) -> Vec<PathBuf> {
    crate::commands::walk_ignored_files(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            p.file_name()
                .map(|n| {
                    crate::providers::makefile::is_standard_makefile_name(&n.to_string_lossy())
                })
                .unwrap_or(false)
        })
        .collect()
}

/// The standard Makefile directly inside `dir` (priority: Makefile > makefile >
/// GNUmakefile), or `None`.
fn own_makefile(dir: &str) -> Option<PathBuf> {
    for name in crate::providers::makefile::STANDARD_MAKEFILE_NAMES {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// First path component of `file` relative to `dir` when `file` lives in a
/// subdirectory (not directly in `dir`). A file sitting directly in `dir` is a
/// sibling, not a child, and yields `None`.
fn first_subdir_segment(file: &Path, dir: &str) -> Option<String> {
    let f_norm = normalize_path(&file.to_string_lossy());
    let rel = Path::new(&f_norm).strip_prefix(dir).ok()?;
    let mut comps = rel.components();
    let first = comps.next()?.as_os_str().to_string_lossy().to_string();
    comps.next()?; // require a deeper component → `file` is under a subdir
    Some(first)
}

/// Fold this directory's Makefile targets into `group` as make-sourced tasks.
/// Rig-authored tasks already on the group win: a make target whose name is
/// already taken is skipped (precedence falls out of gathering rig first).
fn add_make_tasks(group: &mut Group, dir: &str) {
    let mf = match own_makefile(dir) {
        Some(p) => p,
        None => return,
    };
    let taken: HashSet<String> = group.tasks.iter().map(|(n, _)| n.clone()).collect();
    let (targets, goal) = crate::providers::makefile::parse_makefile_with_goal(&mf);
    for (target, description) in targets {
        if taken.contains(&target) {
            continue;
        }
        let command = crate::providers::makefile::make_command(&mf, &target);
        let default_goal = goal.as_deref() == Some(target.as_str());
        group.tasks.push((
            target,
            TaskDef {
                command,
                working_dir: Some(dir.to_string()),
                environment: None,
                env_file: None,
                env_files: Vec::new(),
                description,
                source: TaskSource::Make,
                default_goal,
            },
        ));
    }
}

/// Build a group backed by a directory: its own rig file (bare units + authored
/// child groups + props) plus folder-auto child groups for subdirs that hold a
/// rig file and aren't already adopted by an authored `dir:`/`paths:` group.
fn build_dir_group(
    name: &str,
    dir: &str,
    visited: &mut HashSet<String>,
) -> Result<Group, ConfigError> {
    let dir_norm = normalize_path(dir);
    if !visited.insert(dir_norm.clone()) {
        return Ok(Group::empty(name, Some(PathBuf::from(&dir_norm))));
    }

    let mut group = Group::empty(name, Some(PathBuf::from(&dir_norm)));
    let mut adopted_dirs: HashSet<String> = HashSet::new();
    let mut adopted_files: HashSet<String> = HashSet::new();

    if let Some(file) = pick_config_in_dir(&dir_norm) {
        let file_str = file.to_string_lossy().to_string();
        adopted_files.insert(normalize_path(&file_str));
        let parsed = parse_file(&file_str)?;
        group.props = parsed.props;
        group.tasks = parsed.tasks;
        group.services = parsed.services;
        for (cname, cbody) in parsed.child_groups {
            let child = build_child_group(
                &cname,
                &cbody,
                &parsed.dir,
                &mut adopted_dirs,
                &mut adopted_files,
                visited,
            )?;
            group.groups.push(child);
        }
    }

    // This directory's own Makefile targets (rig gathered first → rig wins).
    add_make_tasks(&mut group, &dir_norm);

    // Folder-auto child groups: immediate subdirs holding a non-adopted rig file
    // OR a Makefile. Namespacing is the relative folder path, identical for
    // make-only and rig-only monorepos.
    let mut subdirs: BTreeSet<String> = BTreeSet::new();
    for f in &scan_rig_files(&dir_norm) {
        let f_norm = normalize_path(&f.to_string_lossy());
        if adopted_files.contains(&f_norm) {
            continue;
        }
        if let Some(seg) = first_subdir_segment(f, &dir_norm) {
            subdirs.insert(seg);
        }
    }
    for f in &scan_makefiles(&dir_norm) {
        if let Some(seg) = first_subdir_segment(f, &dir_norm) {
            subdirs.insert(seg);
        }
    }

    for seg in subdirs {
        let sub = normalize_path(&format!("{}/{}", dir_norm, seg));
        if adopted_dirs.contains(&sub) {
            continue;
        }
        if group.groups.iter().any(|c| c.name == seg) {
            continue; // authored group of the same segment wins
        }
        group.groups.push(build_dir_group(&seg, &sub, visited)?);
    }

    Ok(group)
}

/// Overlay rig-authored tasks onto a group's task list, rig winning on a name
/// clash: any existing make-sourced task with a name being added is dropped
/// first (a `dir:` group's inline/`paths:` rig task beats the dir's Makefile
/// target). Rig-vs-rig collisions keep both, as before (resolution reports the
/// ambiguity).
fn overlay_rig_tasks(dst: &mut Vec<(String, TaskDef)>, add: Vec<(String, TaskDef)>) {
    let adding: HashSet<String> = add.iter().map(|(n, _)| n.clone()).collect();
    dst.retain(|(n, d)| !(d.source == TaskSource::Make && adding.contains(n)));
    dst.extend(add);
}

/// Build an authored child group from its inline body. `dir:` recurses into a
/// directory; `paths:` pulls explicit files; inline units/props apply on top.
fn build_child_group(
    name: &str,
    body: &serde_yaml::Value,
    parent_dir: &str,
    adopted_dirs: &mut HashSet<String>,
    adopted_files: &mut HashSet<String>,
    visited: &mut HashSet<String>,
) -> Result<Group, ConfigError> {
    // A group body may point at a directory. Build that first, then overlay.
    let dir_ref = body
        .as_mapping()
        .and_then(|m| m.get("dir"))
        .and_then(|v| v.as_str());

    let mut group = if let Some(d) = dir_ref {
        let cd = resolve_path(d, parent_dir);
        adopted_dirs.insert(cd.clone());
        let mut g = build_dir_group(name, &cd, visited)?;
        g.dir = Some(PathBuf::from(&cd));
        g
    } else {
        Group::empty(name, None)
    };

    // Inline body (props / units / paths / nested groups) resolved relative to
    // the declaring file's directory, overlaid on top of any `dir:` content.
    let parsed = parse_level(body, name, parent_dir, parent_dir)?;
    if parsed.props.working_dir.is_some() {
        group.props.working_dir = parsed.props.working_dir;
    }
    group.props.env.extend(parsed.props.env);
    group.props.env_files.extend(parsed.props.env_files);
    overlay_rig_tasks(&mut group.tasks, parsed.tasks);
    group.services.extend(parsed.services);

    // Explicit `paths:` files pulled into this group.
    if let Some(paths) = body
        .as_mapping()
        .and_then(|m| m.get("paths"))
        .and_then(|v| v.as_sequence())
    {
        let mut path_bufs = Vec::new();
        for p in paths {
            let ps = match p.as_str() {
                Some(s) => s,
                None => continue,
            };
            let pf = resolve_path(ps, parent_dir);
            adopted_files.insert(pf.clone());
            path_bufs.push(PathBuf::from(&pf));
            let pparsed = parse_file(&pf)?;
            group.props.env.extend(pparsed.props.env);
            group.props.env_files.extend(pparsed.props.env_files);
            overlay_rig_tasks(&mut group.tasks, pparsed.tasks);
            group.services.extend(pparsed.services);
            for (cname, cbody) in pparsed.child_groups {
                let child = build_child_group(
                    &cname,
                    &cbody,
                    &pparsed.dir,
                    adopted_dirs,
                    adopted_files,
                    visited,
                )?;
                group.groups.push(child);
            }
        }
        group.paths = Some(path_bufs);
    }

    // Nested authored `groups:` in the inline body.
    for (cname, cbody) in parsed.child_groups {
        if group.groups.iter().any(|c| c.name == cname) {
            continue;
        }
        let child = build_child_group(
            &cname,
            &cbody,
            parent_dir,
            adopted_dirs,
            adopted_files,
            visited,
        )?;
        group.groups.push(child);
    }

    Ok(group)
}

/// Validate every service's `depends_on` against the set of known service names.
fn validate_depends_on(root: &Group) -> Result<(), ConfigError> {
    let mut names: HashSet<String> = HashSet::new();
    collect_service_names(root, &mut names);

    fn walk(g: &Group, prefix: &str, names: &HashSet<String>) -> Result<(), ConfigError> {
        for (sname, sdef) in &g.services {
            if let Some(deps) = &sdef.depends_on {
                for dep in deps {
                    if !names.contains(dep) {
                        return Err(ConfigError::generic(format!(
                            "Service '{}' depends on unknown service '{}'",
                            join_path(prefix, sname),
                            dep
                        )));
                    }
                }
            }
        }
        for child in &g.groups {
            walk(child, &join_path(prefix, &child.name), names)?;
        }
        Ok(())
    }
    walk(root, "", &names)
}

fn collect_service_names(g: &Group, out: &mut HashSet<String>) {
    for (sname, _) in &g.services {
        out.insert(sname.clone());
    }
    for child in &g.groups {
        collect_service_names(child, out);
    }
}

// ============================================================================
// CONFIG LOADING
// ============================================================================

/// Build the group tree rooted at `config_path`'s directory (or CWD). Returns
/// `Ok(None)` when no rig config exists anywhere under the root (make-only is
/// still valid); a malformed config still fails loudly.
pub fn try_load_config(config_path: Option<&str>) -> Result<Option<(Group, String)>, ConfigError> {
    let base = match config_path {
        Some(p) => Path::new(p)
            .parent()
            .map(|d| d.to_string_lossy().to_string())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| ".".to_string()),
        None => std::env::current_dir()
            .map_err(|e| ConfigError::generic(format!("Failed to get current directory: {}", e)))?
            .to_string_lossy()
            .to_string(),
    };
    let base = normalize_path(&base);

    // The entry directory's own config/Makefile is found directly (regardless of
    // gitignore) — a project may gitignore its rig.yaml (secrets) yet still expect
    // rig to use it when run from that directory. Only downward sub-config discovery
    // is gitignore-aware.
    let has_own_config = pick_config_in_dir(&base).is_some();
    let has_own_makefile = crate::providers::makefile::STANDARD_MAKEFILE_NAMES
        .iter()
        .any(|n| Path::new(&base).join(n).is_file());

    // Make-only is still valid: build the tree when either a rig file or a
    // Makefile exists in the entry dir or anywhere discoverable under it.
    if !has_own_config
        && !has_own_makefile
        && scan_rig_files(&base).is_empty()
        && scan_makefiles(&base).is_empty()
    {
        return Ok(None);
    }

    log_verbose(&format!("config_root={}", base));
    let mut visited = HashSet::new();
    let root = build_dir_group("", &base, &mut visited)?;
    validate_depends_on(&root)?;
    Ok(Some((root, base)))
}

/// Build the group tree; error if no rig config is found.
pub fn load_config(config_path: Option<&str>) -> Result<(Group, String), ConfigError> {
    try_load_config(config_path)?.ok_or_else(|| {
        ConfigError::generic(
            "No rig config found. Expected rig.yaml, rig.yml, or *.rig.yaml under the current directory.",
        )
    })
}

// ============================================================================
// ENV CASCADE + FLATTENING
// ============================================================================

fn join_path(prefix: &str, seg: &str) -> String {
    if prefix.is_empty() {
        seg.to_string()
    } else {
        format!("{}.{}", prefix, seg)
    }
}

/// Fold a unit's own env under the ancestor chain (root-first). Ancestors
/// override the unit; nearer-to-root overrides nearer-to-unit (control from
/// above).
fn cascade_env(
    base: HashMap<String, String>,
    chain: &[&HashMap<String, String>],
) -> HashMap<String, String> {
    let mut eff = base;
    for env in chain.iter().rev() {
        for (k, v) in env.iter() {
            eff.insert(k.clone(), v.clone());
        }
    }
    eff
}

/// Walk `group_path` (dotted, `""` = root) from `root`, returning the leaf group
/// and the chain of its `Props` root→leaf. `None` if a segment doesn't resolve.
fn walk_group_chain<'a>(root: &'a Group, group_path: &str) -> Option<(&'a Group, Vec<&'a Props>)> {
    let mut chain: Vec<&Props> = vec![&root.props];
    let mut cur = root;
    if !group_path.is_empty() {
        for seg in group_path.split('.') {
            cur = cur.groups.iter().find(|g| g.name == seg)?;
            chain.push(&cur.props);
        }
    }
    Some((cur, chain))
}

/// Fold a unit's own materialized env under the group-ancestry chain, loading
/// each level's env files. Ancestor wins (root last), preserving the exact
/// parse-time precedence: unit.env_file < unit.inline < leaf.env_file <
/// leaf.inline < … < root.env_file < root.inline. Fail fast on a
/// required-missing file anywhere in the chain.
fn cascade_materialize(
    base: HashMap<String, String>,
    chain: &[&Props],
) -> Result<HashMap<String, String>, ConfigError> {
    let mut eff = base;
    for props in chain.iter().rev() {
        let level = fold_level_env(&props.env_files, &props.env)?;
        for (k, v) in level {
            eff.insert(k, v);
        }
    }
    Ok(eff)
}

/// Materialize a task's effective env at run time: load its env-file chain and
/// fold with the ancestor-wins cascade. A required-missing file for the run
/// target fails fast here. Returns `None` when the result is empty.
pub fn materialize_task_env(
    root: &Group,
    group_path: &str,
    service: Option<&str>,
    task_name: &str,
) -> Result<Option<HashMap<String, String>>, ConfigError> {
    let (grp, chain) = walk_group_chain(root, group_path)
        .ok_or_else(|| ConfigError::generic(format!("Unknown group '{}'", group_path)))?;
    let empty = HashMap::new();

    let base = match service {
        None => {
            let (_, tdef) = grp
                .tasks
                .iter()
                .find(|(n, _)| n == task_name)
                .ok_or_else(|| ConfigError::generic(format!("Unknown task '{}'", task_name)))?;
            fold_level_env(&tdef.env_files, tdef.environment.as_ref().unwrap_or(&empty))?
        }
        Some(svc) => {
            let (_, sdef) = grp
                .services
                .iter()
                .find(|(n, _)| n == svc)
                .ok_or_else(|| ConfigError::generic(format!("Unknown service '{}'", svc)))?;
            let tdef = sdef
                .tasks
                .as_ref()
                .and_then(|ts| ts.get(task_name))
                .ok_or_else(|| ConfigError::generic(format!("Unknown task '{}'", task_name)))?;
            let mut b =
                fold_level_env(&sdef.env_files, sdef.environment.as_ref().unwrap_or(&empty))?;
            let te = fold_level_env(&tdef.env_files, tdef.environment.as_ref().unwrap_or(&empty))?;
            b.extend(te);
            b
        }
    };

    let eff = cascade_materialize(base, &chain)?;
    Ok(if eff.is_empty() { None } else { Some(eff) })
}

/// Materialize a service's effective env at start time: same ancestor-wins
/// cascade as tasks, loading the env-file chain. Fail fast on required-missing.
pub fn materialize_service_env(
    root: &Group,
    group_path: &str,
    service_name: &str,
) -> Result<Option<HashMap<String, String>>, ConfigError> {
    let (grp, chain) = walk_group_chain(root, group_path)
        .ok_or_else(|| ConfigError::generic(format!("Unknown group '{}'", group_path)))?;
    let (_, sdef) = grp
        .services
        .iter()
        .find(|(n, _)| n == service_name)
        .ok_or_else(|| ConfigError::generic(format!("Unknown service '{}'", service_name)))?;
    let empty = HashMap::new();
    let base = fold_level_env(&sdef.env_files, sdef.environment.as_ref().unwrap_or(&empty))?;
    let eff = cascade_materialize(base, &chain)?;
    Ok(if eff.is_empty() { None } else { Some(eff) })
}

/// Materialize env in place for each target service before start. Fail fast on a
/// required-missing env file for any target being started.
pub fn materialize_targets_env(
    root: &Group,
    targets: &mut [ResolvedService],
) -> Result<(), ConfigError> {
    for t in targets.iter_mut() {
        t.def.environment = materialize_service_env(root, &t.group, &t.name)?;
    }
    Ok(())
}

/// All services as a flat, sorted vector with dotted-path groups and the
/// ancestor-wins env cascade baked into each `def.environment`.
pub fn get_all_services(root: &Group) -> Vec<ResolvedService> {
    let mut out = Vec::new();
    collect_services(root, "", &mut Vec::new(), &mut out);
    out
}

fn collect_services<'a>(
    g: &'a Group,
    prefix: &str,
    chain: &mut Vec<&'a HashMap<String, String>>,
    out: &mut Vec<ResolvedService>,
) {
    chain.push(&g.props.env);

    let mut svcs: Vec<&(String, ServiceDef)> = g.services.iter().collect();
    svcs.sort_by(|a, b| a.0.cmp(&b.0));
    for (sname, sdef) in svcs {
        let base = sdef.environment.clone().unwrap_or_default();
        let eff = cascade_env(base, chain);
        let mut def = sdef.clone();
        def.environment = if eff.is_empty() { None } else { Some(eff) };
        out.push(ResolvedService {
            group: prefix.to_string(),
            name: sname.clone(),
            def,
        });
    }

    let mut children: Vec<&Group> = g.groups.iter().collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));
    for child in children {
        collect_services(child, &join_path(prefix, &child.name), chain, out);
    }

    chain.pop();
}

/// All tasks as a flat, sorted vector with dotted paths and cascaded env.
pub fn get_all_tasks(root: &Group) -> Vec<ResolvedTask> {
    let mut out = Vec::new();
    collect_tasks(root, "", &mut Vec::new(), &mut out);
    out
}

fn collect_tasks<'a>(
    g: &'a Group,
    prefix: &str,
    chain: &mut Vec<&'a HashMap<String, String>>,
    out: &mut Vec<ResolvedTask>,
) {
    chain.push(&g.props.env);

    // Group-level tasks.
    let mut tasks: Vec<&(String, TaskDef)> = g.tasks.iter().collect();
    tasks.sort_by(|a, b| a.0.cmp(&b.0));
    for (tname, tdef) in tasks {
        let base = tdef.environment.clone().unwrap_or_default();
        let eff = cascade_env(base, chain);
        out.push(ResolvedTask {
            path: join_path(prefix, tname),
            group: prefix.to_string(),
            service: None,
            name: tname.clone(),
            command: tdef.command.clone(),
            working_dir: tdef.working_dir.clone().unwrap_or_default(),
            environment: if eff.is_empty() { None } else { Some(eff) },
            description: tdef.description.clone(),
            source: tdef.source,
            default_goal: tdef.default_goal,
        });
    }

    // Service-level tasks (service env merged first, task overrides; then cascade).
    let mut svcs: Vec<&(String, ServiceDef)> = g.services.iter().collect();
    svcs.sort_by(|a, b| a.0.cmp(&b.0));
    for (sname, sdef) in svcs {
        if let Some(svc_tasks) = &sdef.tasks {
            let mut st: Vec<(&String, &TaskDef)> = svc_tasks.iter().collect();
            st.sort_by(|a, b| a.0.cmp(b.0));
            for (tname, tdef) in st {
                let mut base = sdef.environment.clone().unwrap_or_default();
                if let Some(te) = &tdef.environment {
                    base.extend(te.clone());
                }
                let eff = cascade_env(base, chain);
                out.push(ResolvedTask {
                    path: format!("{}.{}", join_path(prefix, sname), tname),
                    group: prefix.to_string(),
                    service: Some(sname.clone()),
                    name: tname.clone(),
                    command: tdef.command.clone(),
                    working_dir: tdef
                        .working_dir
                        .clone()
                        .unwrap_or_else(|| sdef.working_dir.clone()),
                    environment: if eff.is_empty() { None } else { Some(eff) },
                    description: tdef.description.clone(),
                    source: TaskSource::Rig,
                    default_goal: false,
                });
            }
        }
    }

    let mut children: Vec<&Group> = g.groups.iter().collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));
    for child in children {
        collect_tasks(child, &join_path(prefix, &child.name), chain, out);
    }

    chain.pop();
}

/// Set of every group's dotted path (excluding the root `""`).
fn collect_group_paths(root: &Group) -> HashSet<String> {
    let mut out = HashSet::new();
    fn walk(g: &Group, prefix: &str, out: &mut HashSet<String>) {
        for child in &g.groups {
            let p = join_path(prefix, &child.name);
            out.insert(p.clone());
            walk(child, &p, out);
        }
    }
    walk(root, "", &mut out);
    out
}

// ============================================================================
// CONFIG QUERYING
// ============================================================================

/// Flat lookup from bare service name to its resolved form (last one wins on a
/// name collision across groups).
pub fn build_service_lookup(root: &Group) -> HashMap<String, ResolvedService> {
    get_all_services(root)
        .into_iter()
        .map(|s| (s.name.clone(), s))
        .collect()
}

/// Resolve a task path (`task`, `group.task`, or `group.service.task`).
pub fn resolve_task(path: &str, root: &Group) -> Result<ResolvedTask, ConfigError> {
    // Listing builds the tree WITHOUT loading env files; env for the chosen run
    // target is materialized below (fail-fast on a required-missing file).
    let all = get_all_tasks(root);

    let mut found = if !path.contains('.') {
        let matches: Vec<_> = all.into_iter().filter(|t| t.name == path).collect();
        match matches.len() {
            0 => return Err(ConfigError::generic(format!("Unknown task '{}'", path))),
            1 => matches.into_iter().next().unwrap(),
            _ => {
                let mut paths: Vec<_> = matches.iter().map(|t| t.path.clone()).collect();
                paths.sort();
                return Err(ConfigError::generic(format!(
                    "Ambiguous task '{}'. Matches: {}",
                    path,
                    paths.join(", ")
                )));
            }
        }
    } else {
        match all.into_iter().find(|t| t.path == path) {
            Some(t) => t,
            None => {
                // No exact match — distinguish an unknown top-level group from a
                // missing task.
                let first = path.split('.').next().unwrap_or("");
                if !root.groups.iter().any(|c| c.name == first) {
                    return Err(ConfigError::generic(format!("Unknown group '{}'", first)));
                }
                return Err(ConfigError::generic(format!(
                    "Unknown task '{}'. Did you mean 'group.service.task'?",
                    path
                )));
            }
        }
    };

    found.environment =
        materialize_task_env(root, &found.group, found.service.as_deref(), &found.name)?;
    Ok(found)
}

/// Resolve CLI targets (group filters, then explicit service names, then all).
pub fn resolve_targets(
    root: &Group,
    lookup: &HashMap<String, ResolvedService>,
    service_names: &[String],
    group_names: &[String],
) -> Result<Vec<ResolvedService>, ConfigError> {
    if !group_names.is_empty() {
        let known = collect_group_paths(root);
        let all = get_all_services(root);
        let mut services = Vec::new();
        for group_name in group_names {
            if !known.contains(group_name) {
                return Err(ConfigError::generic(format!(
                    "Unknown group: {}",
                    group_name
                )));
            }
            services.extend(all.iter().filter(|s| &s.group == group_name).cloned());
        }
        return Ok(services);
    }

    if !service_names.is_empty() {
        let mut services = Vec::new();
        for name in service_names {
            let resolved = lookup
                .get(name)
                .ok_or_else(|| ConfigError::generic(format!("Unknown service: {}", name)))?;
            services.push(resolved.clone());
        }
        return Ok(services);
    }

    Ok(get_all_services(root))
}
