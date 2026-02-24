use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::sync::LazyLock;

use regex::Regex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::config::{RequirementDef, ResolvedService, ServiceDef, WatchDef, LOG_DIR};
use crate::output::{c, log, log_system, log_verbose, print, strip_control_codes, SERVICE_COLORS};

// ============================================================================
// TYPES
// ============================================================================

#[derive(Debug, Clone)]
pub struct SessionStatus {
    pub name: String,
    pub running: bool,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub created: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ProcessMetrics {
    pub memory_mb: u64,
    pub cpu_percent: f64,
    pub ports: Vec<u16>,
    pub process_count: usize,
}

// ============================================================================
// PROCESS INSPECTION
// ============================================================================

pub async fn get_process_tree(root_pid: u32) -> Vec<u32> {
    let mut all_pids = HashSet::new();
    all_pids.insert(root_pid);
    let mut to_check = vec![root_pid];

    while let Some(parent_pid) = to_check.pop() {
        match Command::new("pgrep")
            .args(["-P", &parent_pid.to_string()])
            .output()
            .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if let Ok(pid) = line.trim().parse::<u32>() {
                        if all_pids.insert(pid) {
                            to_check.push(pid);
                        }
                    }
                }
            }
            Err(e) => {
                log_verbose(&format!("pgrep failed for pid {}: {}", parent_pid, e));
            }
        }
    }

    all_pids.into_iter().collect()
}

static PORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r":(\d+)\s+\(LISTEN\)").expect("port regex is valid"));

pub async fn get_process_metrics(root_pid: u32) -> ProcessMetrics {
    let pids = get_process_tree(root_pid).await;
    let mut total_memory_kb: u64 = 0;
    let mut total_cpu: f64 = 0.0;
    let mut ports = HashSet::new();

    if !pids.is_empty() {
        let pid_strs: Vec<String> = pids.iter().map(|p| p.to_string()).collect();

        // Get memory and CPU via ps: columns are PID, RSS (kb), %CPU
        if let Ok(output) = Command::new("ps")
            .args(["-o", "pid,rss,%cpu", "-p", &pid_strs.join(",")])
            .output()
            .await
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let parts: Vec<&str> = line.trim().split_whitespace().collect();
                if parts.len() >= 3 {
                    total_memory_kb += parts[1].parse::<u64>().unwrap_or(0);
                    total_cpu += parts[2].parse::<f64>().unwrap_or(0.0);
                }
            }
        }

        // Get listening ports via lsof
        if let Ok(output) = Command::new("lsof").args(["-i", "-P", "-n"]).output().await {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let pid_set: HashSet<String> = pid_strs.into_iter().collect();

            for line in stdout.lines() {
                if line.contains("LISTEN") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 && pid_set.contains(parts[1]) {
                        if let Some(caps) = PORT_RE.captures(line) {
                            if let Ok(port) = caps[1].parse::<u16>() {
                                ports.insert(port);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut port_vec: Vec<u16> = ports.into_iter().collect();
    port_vec.sort();

    ProcessMetrics {
        memory_mb: total_memory_kb / 1024,
        cpu_percent: (total_cpu * 10.0).round() / 10.0,
        ports: port_vec,
        process_count: pids.len(),
    }
}

// ============================================================================
// TMUX CHECK
// ============================================================================

pub async fn check_tmux() -> bool {
    Command::new("which")
        .arg("tmux")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn print_tmux_install_guide() {
    print(&format!(
        "\n{}Error: tmux is not installed{}\n\ntmux is required to manage background processes.\n\nInstall:\n  macOS:         brew install tmux\n  Ubuntu/Debian: sudo apt install tmux\n  Fedora:        sudo dnf install tmux\n  Arch:          sudo pacman -S tmux\n",
        c("red"), c("reset")
    ));
}

// ============================================================================
// COMMAND BUILDING HELPERS
// ============================================================================

fn shell_quote(s: &str) -> String {
    if s.contains(|c: char| {
        matches!(
            c,
            '*' | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '$'
                | '`'
                | '"'
                | '\''
                | '\\'
                | '!'
                | '<'
                | '>'
                | '|'
                | ';'
                | '&'
                | '('
                | ')'
                | ' '
                | '\t'
                | '\n'
        )
    }) {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

fn build_watchexec_command(command: &str, watch: &WatchDef, working_dir: &str) -> String {
    let mut args = Vec::new();

    let paths = if let Some(ref p) = watch.paths {
        if p.is_empty() {
            vec![working_dir.to_string()]
        } else {
            p.clone()
        }
    } else {
        vec![working_dir.to_string()]
    };
    for p in &paths {
        args.push("-w".to_string());
        args.push(shell_quote(p));
    }

    if let Some(ref exts) = watch.extensions {
        if !exts.is_empty() {
            args.push("-e".to_string());
            args.push(exts.join(","));
        }
    }

    if let Some(ref patterns) = watch.patterns {
        for p in patterns {
            args.push("--filter".to_string());
            args.push(shell_quote(p));
        }
    }

    if let Some(ref ignore) = watch.ignore {
        for i in ignore {
            args.push("--ignore".to_string());
            args.push(shell_quote(i));
        }
    }

    if let Some(ref debounce) = watch.debounce {
        args.push("--debounce".to_string());
        args.push(debounce.clone());
    }

    args.push("--restart".to_string());
    args.push("--".to_string());
    args.push(command.to_string());

    format!("watchexec {}", args.join(" "))
}

fn build_env_string(env: &HashMap<String, String>) -> String {
    let mut pairs: Vec<_> = env.iter().collect();
    pairs.sort_by_key(|(k, _)| *k);
    pairs
        .into_iter()
        .map(|(k, v)| {
            let escaped = v.replace('\\', "\\\\").replace('"', "\\\"");
            format!("{}=\"{}\"", k, escaped)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn ensure_gitignore(config_dir: &str) {
    let gitignore_path = format!("{}/.gitignore", config_dir);
    let path = Path::new(&gitignore_path);

    if let Ok(content) = std::fs::read_to_string(path) {
        if content.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == ".rig" || trimmed == ".rig/"
        }) {
            return;
        }
        let new_content = if content.ends_with('\n') {
            format!("{}.rig/\n", content)
        } else {
            format!("{}\n.rig/\n", content)
        };
        if let Err(e) = std::fs::write(path, new_content) {
            log_verbose(&format!("Failed to update .gitignore: {}", e));
        }
    } else if let Err(e) = std::fs::write(path, ".rig/\n") {
        log_verbose(&format!("Failed to create .gitignore: {}", e));
    }
}

// ============================================================================
// PRE-START CHECKS
// ============================================================================

pub async fn check_requirements(
    service: &str,
    requirements: &[RequirementDef],
    working_dir: &str,
    environment: &Option<HashMap<String, String>>,
    remediated: &mut HashSet<String>,
) -> Result<(), String> {
    for req in requirements {
        // Run the check command
        let mut check_cmd = Command::new("sh");
        check_cmd.args(["-c", &req.check]);
        check_cmd.current_dir(working_dir);
        check_cmd.stdout(Stdio::null());
        check_cmd.stderr(Stdio::null());
        if let Some(env) = environment {
            check_cmd.envs(env);
        }

        let check_result = check_cmd.output().await.map_err(|e| {
            format!(
                "Failed to run requirement check '{}' for {}: {}",
                req.check, service, e
            )
        })?;

        if check_result.status.success() {
            continue;
        }

        // Check if already remediated
        if remediated.contains(&req.check) {
            let mut recheck = Command::new("sh");
            recheck.args(["-c", &req.check]);
            recheck.current_dir(working_dir);
            recheck.stdout(Stdio::null());
            recheck.stderr(Stdio::null());
            if let Some(env) = environment {
                recheck.envs(env);
            }
            let recheck_result = recheck
                .output()
                .await
                .map_err(|e| format!("Failed to recheck requirement for {}: {}", service, e))?;
            if recheck_result.status.success() {
                continue;
            }
            return Err(format!(
                "Requirement check failed for {}: '{}' (already remediated, still failing)",
                service, req.check
            ));
        }

        // Run remediation
        log_system(&format!(
            "{}: requirement '{}' not met, running '{}'",
            service, req.check, req.command
        ));
        let mut remediate = Command::new("sh");
        remediate.args(["-c", &req.command]);
        remediate.current_dir(working_dir);
        remediate.stdout(Stdio::inherit());
        remediate.stderr(Stdio::inherit());
        if let Some(env) = environment {
            remediate.envs(env);
        }

        let remediate_result = remediate.output().await.map_err(|e| {
            format!(
                "Failed to run remediation '{}' for {}: {}",
                req.command, service, e
            )
        })?;

        if !remediate_result.status.success() {
            return Err(format!(
                "Requirement remediation failed for {}: '{}' exited with code {}",
                service,
                req.command,
                remediate_result.status.code().unwrap_or(-1)
            ));
        }

        remediated.insert(req.check.clone());
    }
    Ok(())
}

fn check_watchexec_installed() -> Result<(), String> {
    // Synchronous check since this is called during startup
    let output = std::process::Command::new("which")
        .arg("watchexec")
        .output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => Err(format!(
            "\n{}Error: watchexec is not installed{}\n\nwatchexec is required for file watching.\n\nInstall:\n  Installer: curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh -s -- --with-watchexec\n  macOS:     brew install watchexec\n  Arch:      sudo pacman -S watchexec\n  Other:     https://github.com/watchexec/watchexec/releases\n",
            c("red"), c("reset")
        )),
    }
}

// ============================================================================
// SESSION MANAGER
// ============================================================================

#[derive(Debug, Clone)]
pub struct SessionManager {
    pub group: String,
    config_dir: String,
}

impl SessionManager {
    pub fn new(group: &str, config_dir: &str) -> Self {
        SessionManager {
            group: group.to_string(),
            config_dir: config_dir.to_string(),
        }
    }

    pub fn log_dir(&self, service: &str) -> String {
        format!("{}/{}/{}/{}", self.config_dir, LOG_DIR, self.group, service)
    }

    pub fn log_file(&self, service: &str, previous: bool) -> String {
        let name = if previous { "previous" } else { "current" };
        format!("{}/{}.log", self.log_dir(service), name)
    }

    pub async fn rotate_log(&self, service: &str) {
        let dir = self.log_dir(service);
        let current = self.log_file(service, false);
        let previous = self.log_file(service, true);

        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            log_verbose(&format!("Failed to create log dir {}: {}", dir, e));
            return;
        }
        ensure_gitignore(&self.config_dir).await;

        // Rotate current -> previous (ok if current doesn't exist yet)
        if let Err(e) = tokio::fs::rename(&current, &previous).await {
            log_verbose(&format!("Log rotate (expected on first start): {}", e));
        }
        if let Err(e) = tokio::fs::write(&current, "").await {
            log_verbose(&format!("Failed to create log file {}: {}", current, e));
        }
    }

    pub fn session_name(&self, service: &str) -> String {
        format!("{}-{}", self.group, service)
    }

    pub async fn start(
        &self,
        service: &str,
        def: &ServiceDef,
        remediated: &mut HashSet<String>,
    ) -> Result<(), String> {
        let session = self.session_name(service);

        // Check if already running
        if self.exists(service).await {
            let status = self.status(service).await;
            if status.running {
                log_system(&format!(
                    "{} is already running (pid {:?})",
                    service, status.pid
                ));
                return Ok(());
            }
            // Dead session, kill it first
            self.stop(service).await;
        }

        // Check requirements
        if let Some(reqs) = &def.requirements {
            if !reqs.is_empty() {
                check_requirements(
                    service,
                    reqs,
                    &def.working_dir,
                    &def.environment,
                    remediated,
                )
                .await?;
            }
        }

        // Check for watchexec
        if def.watch.is_some() {
            check_watchexec_installed()?;
        }

        // Build command
        let mut final_command = def.command.clone();
        if let Some(watch) = &def.watch {
            final_command = build_watchexec_command(&def.command, watch, &def.working_dir);
        }

        // Build command with environment
        // Use "export VAR=val; exec cmd" so env vars are available for expansion in cmd
        let cmd = if let Some(env) = &def.environment {
            if env.is_empty() {
                format!("exec {}", final_command)
            } else {
                format!("export {}; exec {}", build_env_string(env), final_command)
            }
        } else {
            format!("exec {}", final_command)
        };

        log_verbose(&format!("command={}", final_command));
        log_verbose(&format!("working_dir={}", def.working_dir));

        // Create tmux session
        let result = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                &session,
                "-c",
                &def.working_dir,
                &cmd,
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to create tmux session for {}: {}", service, e))?;

        if !result.status.success() {
            let err = String::from_utf8_lossy(&result.stderr);
            return Err(format!("Failed to start {}: {}", service, err.trim()));
        }

        // Enable remain-on-exit so we can detect dead processes
        if let Err(e) = Command::new("tmux")
            .args(["set-option", "-t", &session, "remain-on-exit", "on"])
            .output()
            .await
        {
            log_verbose(&format!(
                "Failed to set remain-on-exit for {}: {}",
                service, e
            ));
        }

        // Set up log file
        self.rotate_log(service).await;
        let log_file = self.log_file(service, false);
        if let Err(e) = Command::new("tmux")
            .args([
                "pipe-pane",
                "-t",
                &session,
                "-o",
                &format!("cat >> \"{}\"", log_file),
            ])
            .output()
            .await
        {
            log_verbose(&format!(
                "Failed to set up pipe-pane for {}: {}",
                service, e
            ));
        }

        // Capture any output before pipe-pane was set up
        let existing = self.capture_pane(service).await;
        if !existing.trim().is_empty() {
            if let Err(e) = tokio::fs::write(&log_file, &existing).await {
                log_verbose(&format!(
                    "Failed to write pre-pipe output for {}: {}",
                    service, e
                ));
            }
        }

        // Get PID
        let status = self.status(service).await;
        log_system(&format!("Started {} (pid {:?})", service, status.pid));
        Ok(())
    }

    pub async fn stop(&self, service: &str) {
        let session = self.session_name(service);
        if !self.exists(service).await {
            return;
        }
        log_system(&format!("Stopping {}...", service));
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &session])
            .output()
            .await;
    }

    pub async fn kill_service(&self, service: &str) {
        let session = self.session_name(service);
        if !self.exists(service).await {
            return;
        }
        log_system(&format!("Killing {}...", service));

        let status = self.status(service).await;
        if let Some(pid) = status.pid {
            let pids = get_process_tree(pid).await;
            for pid in &pids {
                let _ = Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .output()
                    .await;
            }
        }

        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &session])
            .output()
            .await;
    }

    pub async fn exists(&self, service: &str) -> bool {
        let session = self.session_name(service);
        Command::new("tmux")
            .args(["has-session", "-t", &session])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub async fn status(&self, service: &str) -> SessionStatus {
        let session = self.session_name(service);

        if !self.exists(service).await {
            return SessionStatus {
                name: service.to_string(),
                running: false,
                pid: None,
                exit_code: None,
                created: None,
            };
        }

        let output = Command::new("tmux")
            .args([
                "display-message",
                "-t",
                &session,
                "-p",
                "#{pane_pid}:#{pane_dead}:#{pane_dead_status}:#{session_created}",
            ])
            .output()
            .await;

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let parts: Vec<&str> = stdout.split(':').collect();
                if parts.len() >= 4 {
                    let pid = parts[0].parse::<u32>().ok();
                    let dead = parts[1] == "1";
                    let exit_code = if dead {
                        parts[2].parse::<i32>().ok()
                    } else {
                        None
                    };
                    let created = parts[3].parse::<u64>().ok();

                    SessionStatus {
                        name: service.to_string(),
                        running: !dead,
                        pid,
                        exit_code,
                        created,
                    }
                } else {
                    SessionStatus {
                        name: service.to_string(),
                        running: false,
                        pid: None,
                        exit_code: None,
                        created: None,
                    }
                }
            }
            Err(e) => {
                log_verbose(&format!("Failed to get status for {}: {}", service, e));
                SessionStatus {
                    name: service.to_string(),
                    running: false,
                    pid: None,
                    exit_code: None,
                    created: None,
                }
            }
        }
    }

    pub async fn list_all(&self) -> Vec<SessionStatus> {
        let output = Command::new("tmux")
            .args([
                "list-sessions",
                "-F",
                "#{session_name}:#{pane_pid}:#{pane_dead}:#{pane_dead_status}:#{session_created}",
            ])
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if stdout.is_empty() {
                    return Vec::new();
                }
                let prefix = format!("{}-", self.group);
                stdout
                    .lines()
                    .filter(|line| line.starts_with(&prefix))
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.split(':').collect();
                        if parts.len() >= 5 {
                            let name = parts[0].strip_prefix(&prefix)?.to_string();
                            let pid = parts[1].parse::<u32>().ok();
                            let dead = parts[2] == "1";
                            let exit_code = if dead {
                                parts[3].parse::<i32>().ok()
                            } else {
                                None
                            };
                            let created = parts[4].parse::<u64>().ok();
                            Some(SessionStatus {
                                name,
                                running: !dead,
                                pid,
                                exit_code,
                                created,
                            })
                        } else {
                            None
                        }
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub async fn capture_pane(&self, service: &str) -> String {
        let session = self.session_name(service);
        if !self.exists(service).await {
            return String::new();
        }

        let output = Command::new("tmux")
            .args(["capture-pane", "-t", &session, "-p", "-S", "-"])
            .output()
            .await;

        match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => String::new(),
        }
    }
}

// ============================================================================
// LOG STREAMING
// ============================================================================

pub struct LogStream {
    handles: Vec<tokio::task::JoinHandle<()>>,
    children: Vec<std::sync::Arc<tokio::sync::Notify>>,
}

impl LogStream {
    pub fn cleanup(&self) {
        for notify in &self.children {
            notify.notify_one();
        }
        for handle in &self.handles {
            handle.abort();
        }
    }
}

pub fn stream_logs(
    managers: &HashMap<String, SessionManager>,
    targets: &[ResolvedService],
    all_services: &[ResolvedService],
    previous: bool,
) -> LogStream {
    let mut handles = Vec::new();
    let mut notifiers = Vec::new();

    for target in targets {
        let mgr = managers.get(&target.group).unwrap();
        let log_file = mgr.log_file(&target.name, previous);
        let color = get_service_color(all_services, &target.name);
        let name = target.name.clone();
        let color_owned = color.to_string();
        let notify = std::sync::Arc::new(tokio::sync::Notify::new());
        let notify_clone = notify.clone();

        let handle = tokio::spawn(async move {
            // tail -F follows by name (handles rotation)
            let mut child = match tokio::process::Command::new("tail")
                .args(["-F", "-s", "0.1", "-n", "+1", &log_file])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    log_verbose(&format!("Failed to tail logs for {}: {}", name, e));
                    return;
                }
            };

            let stdout = match child.stdout.take() {
                Some(s) => s,
                None => return,
            };

            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            loop {
                tokio::select! {
                    _ = notify_clone.notified() => {
                        let _ = child.kill().await;
                        break;
                    }
                    result = lines.next_line() => {
                        match result {
                            Ok(Some(line)) => {
                                let clean = strip_control_codes(&line);
                                if !clean.trim().is_empty() {
                                    log(&clean, &name, &color_owned);
                                }
                            }
                            Ok(None) => break,
                            Err(_) => break,
                        }
                    }
                }
            }
        });

        handles.push(handle);
        notifiers.push(notify);
    }

    LogStream {
        handles,
        children: notifiers,
    }
}

/// Get the service color by index or config override
pub fn get_service_color(all_services: &[ResolvedService], service_name: &str) -> String {
    if let Some(idx) = all_services.iter().position(|s| s.name == service_name) {
        if let Some(color) = &all_services[idx].def.color {
            return color.clone();
        }
        return SERVICE_COLORS[idx % SERVICE_COLORS.len()].to_string();
    }
    SERVICE_COLORS[0].to_string()
}

// ============================================================================
// FACTORY HELPERS
// ============================================================================

pub fn create_managers(
    targets: &[ResolvedService],
    config_dir: &str,
) -> HashMap<String, SessionManager> {
    let mut managers = HashMap::new();
    for target in targets {
        managers
            .entry(target.group.clone())
            .or_insert_with(|| SessionManager::new(&target.group, config_dir));
    }
    managers
}
