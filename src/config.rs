use std::collections::{HashMap, HashSet};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDef {
    pub command: String,
    pub working_dir: Option<String>,
    pub environment: Option<HashMap<String, String>>,
    pub env_file: Option<EnvFileSpec>,
    pub description: Option<String>,
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
    pub environment: Option<HashMap<String, String>>,
    pub env_file: Option<EnvFileSpec>,
    pub color: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub healthcheck: Option<HealthCheck>,
    pub tasks: Option<HashMap<String, TaskDef>>,
    pub watch: Option<WatchDef>,
    pub requirements: Option<Vec<RequirementDef>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GroupDef {
    pub services: Option<HashMap<String, ServiceDef>>,
    pub tasks: Option<HashMap<String, TaskDef>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub groups: HashMap<String, GroupDef>,
}

/// Raw config as parsed from YAML (before processing)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawConfig {
    pub imports: Option<Vec<String>>,
    pub groups: Option<HashMap<String, serde_yaml::Value>>,
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
}

// ============================================================================
// CONSTANTS
// ============================================================================

pub const CONFIG_NAMES: &[&str] = &["rig.yaml", "rig.yml"];
pub const LOG_DIR: &str = ".rig/logs";

// ============================================================================
// SCHEMA VALIDATION
// ============================================================================

/// Known keys at each level of the config hierarchy
fn root_keys() -> HashSet<&'static str> {
    ["imports", "groups"].into_iter().collect()
}
fn group_keys() -> HashSet<&'static str> {
    ["services", "tasks"].into_iter().collect()
}
fn service_keys() -> HashSet<&'static str> {
    [
        "command", "working_dir", "environment", "env_file", "color",
        "depends_on", "healthcheck", "tasks", "watch", "requirements",
    ].into_iter().collect()
}
fn task_keys() -> HashSet<&'static str> {
    ["command", "working_dir", "environment", "env_file", "description"].into_iter().collect()
}
fn watch_keys() -> HashSet<&'static str> {
    ["paths", "extensions", "patterns", "ignore", "debounce"].into_iter().collect()
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
) -> Vec<String> {
    let mut errors = Vec::new();
    for key in obj.keys() {
        if let Some(key_str) = key.as_str() {
            if !allowed.contains(key_str) {
                let mut valid: Vec<&&str> = allowed.iter().collect();
                valid.sort();
                let valid_str: Vec<&str> = valid.into_iter().copied().collect();
                errors.push(format!(
                    "Unknown key '{}' in {} ({}). Valid keys: {}",
                    key_str, context, config_path, valid_str.join(", ")
                ));
            }
        }
    }
    errors
}

fn validate_config_schema(raw: &serde_yaml::Value, config_path: &str) -> Result<(), ConfigError> {
    let mut errors = Vec::new();

    if let Some(root) = raw.as_mapping() {
        errors.extend(validate_keys(root, &root_keys(), "config root", config_path));

        if let Some(groups_val) = root.get("groups") {
            if let Some(groups) = groups_val.as_mapping() {
                for (group_key, group_val) in groups {
                    let group_name = group_key.as_str().unwrap_or("?");
                    if let Some(g) = group_val.as_mapping() {
                        errors.extend(validate_keys(g, &group_keys(), &format!("group '{}'", group_name), config_path));

                        // Validate services
                        if let Some(services_val) = g.get("services") {
                            if let Some(services) = services_val.as_mapping() {
                                for (svc_key, svc_val) in services {
                                    let svc_name = svc_key.as_str().unwrap_or("?");
                                    if let Some(s) = svc_val.as_mapping() {
                                        errors.extend(validate_keys(
                                            s,
                                            &service_keys(),
                                            &format!("service '{}.{}'", group_name, svc_name),
                                            config_path,
                                        ));

                                        // Validate watch
                                        if let Some(watch_val) = s.get("watch") {
                                            if let Some(w) = watch_val.as_mapping() {
                                                errors.extend(validate_keys(
                                                    w,
                                                    &watch_keys(),
                                                    &format!("watch in service '{}.{}'", group_name, svc_name),
                                                    config_path,
                                                ));
                                            }
                                        }

                                        // Validate healthcheck
                                        if let Some(hc_val) = s.get("healthcheck") {
                                            if let Some(hc) = hc_val.as_mapping() {
                                                errors.extend(validate_keys(
                                                    hc,
                                                    &healthcheck_keys(),
                                                    &format!("healthcheck in service '{}.{}'", group_name, svc_name),
                                                    config_path,
                                                ));
                                            }
                                        }

                                        // Validate env_file entries
                                        if let Some(ef_val) = s.get("env_file") {
                                            if let Some(arr) = ef_val.as_sequence() {
                                                for entry in arr {
                                                    if let Some(e) = entry.as_mapping() {
                                                        errors.extend(validate_keys(
                                                            e,
                                                            &env_file_entry_keys(),
                                                            &format!("env_file entry in service '{}.{}'", group_name, svc_name),
                                                            config_path,
                                                        ));
                                                    }
                                                }
                                            }
                                        }

                                        // Validate requirements
                                        if let Some(req_val) = s.get("requirements") {
                                            if let Some(arr) = req_val.as_sequence() {
                                                for entry in arr {
                                                    if let Some(r) = entry.as_mapping() {
                                                        errors.extend(validate_keys(
                                                            r,
                                                            &requirement_keys(),
                                                            &format!("requirement in service '{}.{}'", group_name, svc_name),
                                                            config_path,
                                                        ));
                                                    }
                                                }
                                            }
                                        }

                                        // Validate service-level tasks
                                        if let Some(tasks_val) = s.get("tasks") {
                                            if let Some(tasks) = tasks_val.as_mapping() {
                                                for (tk, tv) in tasks {
                                                    let tname = tk.as_str().unwrap_or("?");
                                                    if let Some(t) = tv.as_mapping() {
                                                        errors.extend(validate_keys(
                                                            t,
                                                            &task_keys(),
                                                            &format!("task '{}.{}.{}'", group_name, svc_name, tname),
                                                            config_path,
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Validate group-level tasks
                        if let Some(tasks_val) = g.get("tasks") {
                            if let Some(tasks) = tasks_val.as_mapping() {
                                for (tk, tv) in tasks {
                                    let tname = tk.as_str().unwrap_or("?");
                                    if let Some(t) = tv.as_mapping() {
                                        errors.extend(validate_keys(
                                            t,
                                            &task_keys(),
                                            &format!("task '{}.{}'", group_name, tname),
                                            config_path,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
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

fn load_env_files(entries: &[ResolvedEnvFileEntry]) -> Result<HashMap<String, String>, ConfigError> {
    let mut result = HashMap::new();

    for entry in entries {
        match std::fs::read_to_string(&entry.path) {
            Ok(content) => {
                // Parse .env file
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    if let Some(eq_pos) = line.find('=') {
                        let key = line[..eq_pos].trim().to_string();
                        let mut val = line[eq_pos + 1..].trim().to_string();
                        // Strip surrounding quotes
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
                // required: false — silently skip
            }
        }
    }
    Ok(result)
}

#[derive(Debug, Clone)]
struct ResolvedEnvFileEntry {
    path: String,
    original_path: String,
    required: bool,
}

fn resolve_env_file_spec(
    spec: &serde_yaml::Value,
    config_dir: &str,
) -> Vec<ResolvedEnvFileEntry> {
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
                            let required = m
                                .get("required")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true);
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

// ============================================================================
// CONFIG LOADING
// ============================================================================

/// Walk upward from start_dir to find the nearest rig config file.
pub fn find_nearest_config(start_dir: &Path) -> Result<PathBuf, ConfigError> {
    let mut dir = std::fs::canonicalize(start_dir).map_err(|e| {
        ConfigError::generic(format!("Cannot resolve path '{}': {}", start_dir.display(), e))
    })?;

    loop {
        // Check standard names first
        for name in CONFIG_NAMES {
            let path = dir.join(name);
            if path.exists() {
                return Ok(path);
            }
        }

        // Check for *.rig.yaml files
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with(".rig.yaml") && name_str != "rig.yaml" {
                    if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                        return Ok(entry.path());
                    }
                }
            }
        }

        // Move to parent
        if let Some(parent) = dir.parent() {
            if parent == dir {
                break;
            }
            dir = parent.to_path_buf();
        } else {
            break;
        }
    }

    Err(ConfigError::generic(
        "No rig config found (searched up to filesystem root). Expected: rig.yaml, rig.yml, or *.rig.yaml",
    ))
}

/// Context for recursive config loading
struct LoadContext {
    loaded: HashSet<String>,
    import_chain: Vec<String>,
}

/// Result of loading a config tree
struct LoadResult {
    groups: HashMap<String, GroupDef>,
    seen_services: HashMap<String, String>, // service_name -> group_name
}

fn load_config_recursive(
    config_path: &str,
    config_dir: &str,
    ctx: &mut LoadContext,
) -> Result<LoadResult, ConfigError> {
    let abs_path = normalize_path(config_path);

    // Circular import check
    if ctx.import_chain.contains(&abs_path) {
        let mut cycle_parts: Vec<String> = ctx.import_chain.iter().map(|p| {
            let cwd = std::env::current_dir().unwrap_or_default();
            let cwd_str = cwd.to_string_lossy();
            if p.starts_with(cwd_str.as_ref()) {
                format!("./{}", &p[cwd_str.len()..].trim_start_matches('/'))
            } else {
                p.clone()
            }
        }).collect();
        let last = {
            let cwd = std::env::current_dir().unwrap_or_default();
            let cwd_str = cwd.to_string_lossy();
            if abs_path.starts_with(cwd_str.as_ref()) {
                format!("./{}", &abs_path[cwd_str.len()..].trim_start_matches('/'))
            } else {
                abs_path.clone()
            }
        };
        cycle_parts.push(last);
        return Err(ConfigError::generic(format!(
            "Circular import detected:\n  {}",
            cycle_parts.join("\n  → ")
        )));
    }

    // Dedup check
    if ctx.loaded.contains(&abs_path) {
        return Ok(LoadResult {
            groups: HashMap::new(),
            seen_services: HashMap::new(),
        });
    }
    ctx.loaded.insert(abs_path.clone());

    // Read and parse YAML
    let content = std::fs::read_to_string(config_path).map_err(|e| {
        ConfigError::generic(format!("Failed to read config file '{}': {}", config_path, e))
    })?;

    let yaml_value: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
        ConfigError::generic(format!("Invalid YAML in {}: {}", config_path, e))
    })?;

    // Schema validation
    validate_config_schema(&yaml_value, config_path)?;

    let raw: RawConfig = serde_yaml::from_value(yaml_value).map_err(|e| {
        ConfigError::generic(format!("Failed to parse config {}: {}", config_path, e))
    })?;

    let mut groups: HashMap<String, GroupDef> = HashMap::new();
    let mut seen_services: HashMap<String, String> = HashMap::new();

    // Process groups
    if let Some(raw_groups) = &raw.groups {
        for (group_name, group_value) in raw_groups {
            // Validate group name
            if !group_name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
                return Err(ConfigError::generic(format!(
                    "Invalid group name '{}' in {}: must be alphanumeric with hyphens/underscores only",
                    group_name, config_path
                )));
            }

            let group_map = group_value.as_mapping().ok_or_else(|| {
                ConfigError::generic(format!("Group '{}' in {} must be a mapping", group_name, config_path))
            })?;

            let has_services = group_map.get("services").is_some();
            let has_tasks = group_map.get("tasks").is_some();

            if !has_services && !has_tasks {
                return Err(ConfigError::generic(format!(
                    "Group '{}' in {} must have 'services' and/or 'tasks'",
                    group_name, config_path
                )));
            }

            let mut services_map: HashMap<String, ServiceDef> = HashMap::new();
            let mut tasks_map: HashMap<String, TaskDef> = HashMap::new();

            // Parse services
            if let Some(services_val) = group_map.get("services") {
                if let Some(services) = services_val.as_mapping() {
                    for (svc_key, svc_val) in services {
                        let svc_name = svc_key.as_str().unwrap_or("?");
                        let s = svc_val.as_mapping().ok_or_else(|| {
                            ConfigError::generic(format!(
                                "Service '{}.{}' in {} must be a mapping",
                                group_name, svc_name, config_path
                            ))
                        })?;

                        // Required fields
                        let command = s.get("command")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| ConfigError::generic(format!(
                                "Service '{}.{}' in {} must have a 'command' field",
                                group_name, svc_name, config_path
                            )))?
                            .to_string();

                        let working_dir_raw = s.get("working_dir")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| ConfigError::generic(format!(
                                "Service '{}.{}' in {} must have a 'working_dir' field",
                                group_name, svc_name, config_path
                            )))?;

                        let working_dir = resolve_path(working_dir_raw, config_dir);

                        // Optional environment
                        let mut environment: Option<HashMap<String, String>> = None;
                        if let Some(env_val) = s.get("environment") {
                            if let Some(env_map) = env_val.as_mapping() {
                                let map: HashMap<String, serde_yaml::Value> = env_map.iter()
                                    .filter_map(|(k, v)| {
                                        k.as_str().map(|ks| (ks.to_string(), v.clone()))
                                    })
                                    .collect();
                                environment = Some(normalize_env_values(&map));
                            }
                        }

                        // env_file
                        let mut env_file_entries = Vec::new();
                        if let Some(ef_val) = s.get("env_file") {
                            env_file_entries = resolve_env_file_spec(ef_val, config_dir);
                        }

                        // Load env files and merge
                        if !env_file_entries.is_empty() {
                            let env_from_files = load_env_files(&env_file_entries)?;
                            let mut merged = env_from_files;
                            if let Some(inline) = &environment {
                                merged.extend(inline.clone());
                            }
                            environment = Some(merged);
                        }

                        // depends_on
                        let depends_on = s.get("depends_on").and_then(|v| {
                            v.as_sequence().map(|seq| {
                                seq.iter()
                                    .filter_map(|item| item.as_str().map(String::from))
                                    .collect::<Vec<String>>()
                            })
                        });

                        // healthcheck
                        let healthcheck = s.get("healthcheck").and_then(|v| {
                            v.as_mapping().map(|m| HealthCheck {
                                grace_ms: m.get("grace_ms").and_then(|v| v.as_u64()),
                            })
                        });

                        // color
                        let color = s.get("color").and_then(|v| v.as_str()).map(String::from);

                        // watch
                        let watch = if let Some(watch_val) = s.get("watch") {
                            if let Some(w) = watch_val.as_mapping() {
                                let mut wd = WatchDef::default();
                                if let Some(paths_val) = w.get("paths") {
                                    if let Some(seq) = paths_val.as_sequence() {
                                        wd.paths = Some(seq.iter().filter_map(|p| {
                                            p.as_str().map(|s| resolve_path(s, &working_dir))
                                        }).collect());
                                    }
                                }
                                if let Some(ext_val) = w.get("extensions") {
                                    if let Some(seq) = ext_val.as_sequence() {
                                        wd.extensions = Some(seq.iter().filter_map(|e| {
                                            e.as_str().map(String::from)
                                        }).collect());
                                    }
                                }
                                if let Some(pat_val) = w.get("patterns") {
                                    if let Some(seq) = pat_val.as_sequence() {
                                        wd.patterns = Some(seq.iter().filter_map(|p| {
                                            p.as_str().map(String::from)
                                        }).collect());
                                    }
                                }
                                if let Some(ign_val) = w.get("ignore") {
                                    if let Some(seq) = ign_val.as_sequence() {
                                        wd.ignore = Some(seq.iter().filter_map(|i| {
                                            i.as_str().map(String::from)
                                        }).collect());
                                    }
                                }
                                if let Some(deb_val) = w.get("debounce") {
                                    wd.debounce = Some(deb_val.as_str()
                                        .map(String::from)
                                        .unwrap_or_else(|| format!("{}", deb_val.as_u64().unwrap_or(0))));
                                }
                                Some(wd)
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        // requirements
                        let requirements = if let Some(req_val) = s.get("requirements") {
                            if let Some(seq) = req_val.as_sequence() {
                                let mut reqs = Vec::new();
                                for entry in seq {
                                    let m = entry.as_mapping().ok_or_else(|| {
                                        ConfigError::generic(format!(
                                            "Invalid requirement entry in service '{}.{}' ({}): must be an object with 'check' and 'command'",
                                            group_name, svc_name, config_path
                                        ))
                                    })?;
                                    let check = m.get("check").and_then(|v| v.as_str()).ok_or_else(|| {
                                        ConfigError::generic(format!(
                                            "Requirement in service '{}.{}' ({}) must have a 'check' string",
                                            group_name, svc_name, config_path
                                        ))
                                    })?.to_string();
                                    let cmd = m.get("command").and_then(|v| v.as_str()).ok_or_else(|| {
                                        ConfigError::generic(format!(
                                            "Requirement in service '{}.{}' ({}) must have a 'command' string",
                                            group_name, svc_name, config_path
                                        ))
                                    })?.to_string();
                                    reqs.push(RequirementDef { check, command: cmd });
                                }
                                Some(reqs)
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        // Service-level tasks
                        let svc_tasks = if let Some(tasks_val) = s.get("tasks") {
                            if let Some(tasks) = tasks_val.as_mapping() {
                                let mut tm = HashMap::new();
                                for (tk, tv) in tasks {
                                    let tname = tk.as_str().unwrap_or("?");
                                    let t = tv.as_mapping().ok_or_else(|| {
                                        ConfigError::generic(format!(
                                            "Task '{}.{}.{}' in {} must be a mapping",
                                            group_name, svc_name, tname, config_path
                                        ))
                                    })?;

                                    let tcmd = t.get("command").and_then(|v| v.as_str())
                                        .ok_or_else(|| ConfigError::generic(format!(
                                            "Task '{}.{}.{}' in {} must have a 'command' field",
                                            group_name, svc_name, tname, config_path
                                        )))?
                                        .to_string();

                                    let twd = t.get("working_dir").and_then(|v| v.as_str())
                                        .map(|wd| resolve_path(wd, config_dir));

                                    let mut tenv: Option<HashMap<String, String>> = None;
                                    if let Some(env_val) = t.get("environment") {
                                        if let Some(env_map) = env_val.as_mapping() {
                                            let map: HashMap<String, serde_yaml::Value> = env_map.iter()
                                                .filter_map(|(k, v)| k.as_str().map(|ks| (ks.to_string(), v.clone())))
                                                .collect();
                                            tenv = Some(normalize_env_values(&map));
                                        }
                                    }

                                    // env_file for task
                                    if let Some(ef_val) = t.get("env_file") {
                                        let ef_entries = resolve_env_file_spec(ef_val, config_dir);
                                        if !ef_entries.is_empty() {
                                            let env_from_files = load_env_files(&ef_entries)?;
                                            let mut merged = env_from_files;
                                            if let Some(inline) = &tenv {
                                                merged.extend(inline.clone());
                                            }
                                            tenv = Some(merged);
                                        }
                                    }

                                    let tdesc = t.get("description").and_then(|v| v.as_str()).map(String::from);

                                    tm.insert(tname.to_string(), TaskDef {
                                        command: tcmd,
                                        working_dir: twd,
                                        environment: tenv,
                                        env_file: None, // already processed
                                        description: tdesc,
                                    });
                                }
                                Some(tm)
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        services_map.insert(svc_name.to_string(), ServiceDef {
                            command,
                            working_dir,
                            environment,
                            env_file: None, // already processed
                            color,
                            depends_on,
                            healthcheck,
                            tasks: svc_tasks,
                            watch,
                            requirements,
                        });

                        seen_services.insert(svc_name.to_string(), group_name.clone());
                    }
                }
            }

            // Parse group-level tasks
            if let Some(tasks_val) = group_map.get("tasks") {
                if let Some(tasks) = tasks_val.as_mapping() {
                    for (tk, tv) in tasks {
                        let tname = tk.as_str().unwrap_or("?");
                        let t = tv.as_mapping().ok_or_else(|| {
                            ConfigError::generic(format!(
                                "Task '{}.{}' in {} must be a mapping",
                                group_name, tname, config_path
                            ))
                        })?;

                        let tcmd = t.get("command").and_then(|v| v.as_str())
                            .ok_or_else(|| ConfigError::generic(format!(
                                "Task '{}.{}' in {} must have a 'command' field",
                                group_name, tname, config_path
                            )))?
                            .to_string();

                        let twd_raw = t.get("working_dir").and_then(|v| v.as_str())
                            .ok_or_else(|| ConfigError::generic(format!(
                                "Task '{}.{}' in {} must have a 'working_dir' field (group-level tasks cannot inherit)",
                                group_name, tname, config_path
                            )))?;
                        let twd = resolve_path(twd_raw, config_dir);

                        let mut tenv: Option<HashMap<String, String>> = None;
                        if let Some(env_val) = t.get("environment") {
                            if let Some(env_map) = env_val.as_mapping() {
                                let map: HashMap<String, serde_yaml::Value> = env_map.iter()
                                    .filter_map(|(k, v)| k.as_str().map(|ks| (ks.to_string(), v.clone())))
                                    .collect();
                                tenv = Some(normalize_env_values(&map));
                            }
                        }

                        if let Some(ef_val) = t.get("env_file") {
                            let ef_entries = resolve_env_file_spec(ef_val, config_dir);
                            if !ef_entries.is_empty() {
                                let env_from_files = load_env_files(&ef_entries)?;
                                let mut merged = env_from_files;
                                if let Some(inline) = &tenv {
                                    merged.extend(inline.clone());
                                }
                                tenv = Some(merged);
                            }
                        }

                        let tdesc = t.get("description").and_then(|v| v.as_str()).map(String::from);

                        tasks_map.insert(tname.to_string(), TaskDef {
                            command: tcmd,
                            working_dir: Some(twd),
                            environment: tenv,
                            env_file: None,
                            description: tdesc,
                        });
                    }
                }
            }

            groups.insert(group_name.clone(), GroupDef {
                services: if services_map.is_empty() { None } else { Some(services_map) },
                tasks: if tasks_map.is_empty() { None } else { Some(tasks_map) },
            });
        }
    }

    // Process imports
    if let Some(imports) = &raw.imports {
        for import_path in imports {
            let abs_import = resolve_path(import_path, config_dir);

            // Check if import exists
            if !Path::new(&abs_import).exists() {
                return Err(ConfigError::generic(format!(
                    "Import not found: {}\n  in {}",
                    import_path, config_path
                )));
            }

            let child_dir = Path::new(&abs_import)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string());

            ctx.import_chain.push(abs_path.clone());
            let child_result = load_config_recursive(&abs_import, &child_dir, ctx)?;
            ctx.import_chain.pop();

            // Merge child groups
            for (group_name, group_def) in child_result.groups {
                if groups.contains_key(&group_name) {
                    return Err(ConfigError::generic(format!(
                        "Duplicate group '{}' defined in:\n  - {}\n  - {}",
                        group_name, config_path, abs_import
                    )));
                }
                groups.insert(group_name, group_def);
            }

            // Merge child services
            for (service_name, group_name) in child_result.seen_services {
                if let Some(existing_group) = seen_services.get(&service_name) {
                    return Err(ConfigError::generic(format!(
                        "Duplicate service '{}' found in:\n  - group '{}'\n  - group '{}' in {}",
                        service_name, existing_group, group_name, abs_import
                    )));
                }
                seen_services.insert(service_name, group_name);
            }
        }
    }

    Ok(LoadResult {
        groups,
        seen_services,
    })
}

/// Load the full config tree starting from a root config file.
fn load_config_tree(root_path: &str) -> Result<(Config, String), ConfigError> {
    let abs_path = if root_path.starts_with('/') {
        root_path.to_string()
    } else {
        let cwd = std::env::current_dir().unwrap_or_default();
        format!("{}/{}", cwd.display(), root_path)
    };
    let abs_path = normalize_path(&abs_path);
    let config_dir = Path::new(&abs_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    let mut ctx = LoadContext {
        loaded: HashSet::new(),
        import_chain: Vec::new(),
    };

    let result = load_config_recursive(&abs_path, &config_dir, &mut ctx)?;

    // Validate depends_on references
    for (group_name, group_def) in &result.groups {
        if let Some(services) = &group_def.services {
            for (svc_name, svc_def) in services {
                if let Some(deps) = &svc_def.depends_on {
                    for dep in deps {
                        if !result.seen_services.contains_key(dep) {
                            return Err(ConfigError::generic(format!(
                                "Service '{}.{}' depends on unknown service '{}'",
                                group_name, svc_name, dep
                            )));
                        }
                    }
                }
            }
        }
    }

    Ok((
        Config { groups: result.groups },
        config_dir,
    ))
}

/// Load config from a path. If no path, searches upward from CWD.
pub fn load_config(config_path: Option<&str>) -> Result<(Config, String), ConfigError> {
    let path = match config_path {
        Some(p) => PathBuf::from(p),
        None => {
            let cwd = std::env::current_dir().map_err(|e| {
                ConfigError::generic(format!("Failed to get current directory: {}", e))
            })?;
            find_nearest_config(&cwd)?
        }
    };
    let path_str = path.to_string_lossy().to_string();
    log_verbose(&format!("config={}", path_str));
    load_config_tree(&path_str)
}

// ============================================================================
// CONFIG QUERYING
// ============================================================================

/// Build a flat lookup map from service name to its group and definition.
pub fn build_service_lookup(config: &Config) -> HashMap<String, ResolvedService> {
    let mut lookup = HashMap::new();
    for (group_name, group_def) in &config.groups {
        if let Some(services) = &group_def.services {
            for (svc_name, svc_def) in services {
                lookup.insert(svc_name.clone(), ResolvedService {
                    group: group_name.clone(),
                    name: svc_name.clone(),
                    def: svc_def.clone(),
                });
            }
        }
    }
    lookup
}

/// Get all services from config as a flat vector.
pub fn get_all_services(config: &Config) -> Vec<ResolvedService> {
    let mut services = Vec::new();
    let mut groups: Vec<_> = config.groups.iter().collect();
    groups.sort_by_key(|(name, _)| *name);
    for (group_name, group_def) in groups {
        if let Some(svcs) = &group_def.services {
            let mut svc_names: Vec<_> = svcs.iter().collect();
            svc_names.sort_by_key(|(name, _)| *name);
            for (svc_name, svc_def) in svc_names {
                services.push(ResolvedService {
                    group: group_name.clone(),
                    name: svc_name.clone(),
                    def: svc_def.clone(),
                });
            }
        }
    }
    services
}

/// Get all tasks from config as a flat vector.
pub fn get_all_tasks(config: &Config) -> Vec<ResolvedTask> {
    let mut tasks = Vec::new();

    let mut groups: Vec<_> = config.groups.iter().collect();
    groups.sort_by_key(|(name, _)| *name);

    for (group_name, group_def) in groups {
        // Group-level tasks
        if let Some(group_tasks) = &group_def.tasks {
            let mut task_names: Vec<_> = group_tasks.iter().collect();
            task_names.sort_by_key(|(name, _)| *name);
            for (task_name, task_def) in task_names {
                tasks.push(ResolvedTask {
                    path: format!("{}.{}", group_name, task_name),
                    group: group_name.clone(),
                    service: None,
                    name: task_name.clone(),
                    command: task_def.command.clone(),
                    working_dir: task_def.working_dir.clone().unwrap_or_default(),
                    environment: task_def.environment.clone(),
                    description: task_def.description.clone(),
                });
            }
        }

        // Service-level tasks
        if let Some(services) = &group_def.services {
            let mut svc_names: Vec<_> = services.iter().collect();
            svc_names.sort_by_key(|(name, _)| *name);
            for (svc_name, svc_def) in svc_names {
                if let Some(svc_tasks) = &svc_def.tasks {
                    let mut task_names: Vec<_> = svc_tasks.iter().collect();
                    task_names.sort_by_key(|(name, _)| *name);
                    for (task_name, task_def) in task_names {
                        // Merge environment: service env -> task env (task overrides)
                        let merged_env = if task_def.environment.is_some() {
                            let mut env = svc_def.environment.clone().unwrap_or_default();
                            env.extend(task_def.environment.clone().unwrap_or_default());
                            Some(env)
                        } else {
                            svc_def.environment.clone()
                        };

                        tasks.push(ResolvedTask {
                            path: format!("{}.{}.{}", group_name, svc_name, task_name),
                            group: group_name.clone(),
                            service: Some(svc_name.clone()),
                            name: task_name.clone(),
                            command: task_def.command.clone(),
                            working_dir: task_def.working_dir.clone().unwrap_or_else(|| svc_def.working_dir.clone()),
                            environment: merged_env,
                            description: task_def.description.clone(),
                        });
                    }
                }
            }
        }
    }

    tasks
}

/// Resolve a task path (e.g., "backend.deploy" or "backend.api.build") to a ResolvedTask.
pub fn resolve_task(path: &str, config: &Config) -> Result<ResolvedTask, ConfigError> {
    let parts: Vec<&str> = path.split('.').collect();

    if parts.len() < 2 || parts.len() > 3 {
        return Err(ConfigError::generic(format!(
            "Invalid task path '{}'. Use 'group.task' or 'group.service.task'",
            path
        )));
    }

    let group_name = parts[0];
    let group_def = config.groups.get(group_name).ok_or_else(|| {
        ConfigError::generic(format!("Unknown group '{}'", group_name))
    })?;

    if parts.len() == 2 {
        let task_name = parts[1];
        if let Some(tasks) = &group_def.tasks {
            if let Some(task_def) = tasks.get(task_name) {
                return Ok(ResolvedTask {
                    path: path.to_string(),
                    group: group_name.to_string(),
                    service: None,
                    name: task_name.to_string(),
                    command: task_def.command.clone(),
                    working_dir: task_def.working_dir.clone().unwrap_or_default(),
                    environment: task_def.environment.clone(),
                    description: task_def.description.clone(),
                });
            }
        }
        return Err(ConfigError::generic(format!(
            "Unknown task '{}'. Did you mean 'group.service.task'?",
            path
        )));
    }

    // parts.len() == 3: group.service.task
    let svc_name = parts[1];
    let task_name = parts[2];

    let svc_def = group_def.services.as_ref()
        .and_then(|svcs| svcs.get(svc_name))
        .ok_or_else(|| ConfigError::generic(format!("Unknown service '{}.{}'", group_name, svc_name)))?;

    let task_def = svc_def.tasks.as_ref()
        .and_then(|tasks| tasks.get(task_name))
        .ok_or_else(|| ConfigError::generic(format!("Unknown task '{}'", path)))?;

    // Merge service env -> task env
    let merged_env = if task_def.environment.is_some() {
        let mut env = svc_def.environment.clone().unwrap_or_default();
        env.extend(task_def.environment.clone().unwrap_or_default());
        Some(env)
    } else {
        svc_def.environment.clone()
    };

    Ok(ResolvedTask {
        path: path.to_string(),
        group: group_name.to_string(),
        service: Some(svc_name.to_string()),
        name: task_name.to_string(),
        command: task_def.command.clone(),
        working_dir: task_def.working_dir.clone().unwrap_or_else(|| svc_def.working_dir.clone()),
        environment: merged_env,
        description: task_def.description.clone(),
    })
}

/// Resolve CLI targets to a list of services.
pub fn resolve_targets(
    config: &Config,
    lookup: &HashMap<String, ResolvedService>,
    service_names: &[String],
    group_names: &[String],
) -> Result<Vec<ResolvedService>, ConfigError> {
    if !group_names.is_empty() {
        let mut services = Vec::new();
        for group_name in group_names {
            let group_def = config.groups.get(group_name).ok_or_else(|| {
                ConfigError::generic(format!("Unknown group: {}", group_name))
            })?;
            if let Some(svcs) = &group_def.services {
                for (svc_name, svc_def) in svcs {
                    services.push(ResolvedService {
                        group: group_name.clone(),
                        name: svc_name.clone(),
                        def: svc_def.clone(),
                    });
                }
            }
        }
        return Ok(services);
    }

    if !service_names.is_empty() {
        let mut services = Vec::new();
        for name in service_names {
            let resolved = lookup.get(name).ok_or_else(|| {
                ConfigError::generic(format!("Unknown service: {}", name))
            })?;
            services.push(resolved.clone());
        }
        return Ok(services);
    }

    // No args — return all services
    Ok(get_all_services(config))
}
