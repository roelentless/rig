use std::collections::HashMap;
use std::io::Write;

use crossterm::{cursor, event, execute, terminal};

use crate::config::{group_tree_to_value, Group, ResolvedService, ResolvedTask};
use crate::output::{
    c, c_raw, log, log_error, log_system, log_verbose, print, strip_control_codes,
};
use crate::process::{get_process_metrics, get_service_color, stream_logs, SessionManager};
use crate::providers::TaskProvider;

// ============================================================================
// DEPENDENCY ORDERING
// ============================================================================

pub fn compute_startup_order(targets: &[ResolvedService]) -> Vec<Vec<ResolvedService>> {
    let requested: std::collections::HashSet<String> =
        targets.iter().map(|t| t.name.clone()).collect();
    let by_name: HashMap<String, &ResolvedService> =
        targets.iter().map(|t| (t.name.clone(), t)).collect();

    // Build dependency graph
    let mut deps: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for target in targets {
        let mut d = std::collections::HashSet::new();
        if let Some(dep_list) = &target.def.depends_on {
            for dep in dep_list {
                if requested.contains(dep) {
                    d.insert(dep.clone());
                }
            }
        }
        deps.insert(target.name.clone(), d);
    }

    // Kahn's algorithm
    let mut levels: Vec<Vec<ResolvedService>> = Vec::new();
    let mut remaining: std::collections::HashSet<String> =
        targets.iter().map(|t| t.name.clone()).collect();

    while !remaining.is_empty() {
        let mut level = Vec::new();
        for name in &remaining {
            let unresolved: Vec<_> = deps[name]
                .iter()
                .filter(|d| remaining.contains(*d))
                .collect();
            if unresolved.is_empty() {
                level.push(by_name[name].clone());
            }
        }

        if level.is_empty() {
            // Circular dependency
            log_system("Warning: circular dependency detected, starting remaining services");
            levels.push(remaining.iter().map(|n| by_name[n].clone()).collect());
            break;
        }

        for svc in &level {
            remaining.remove(&svc.name);
        }
        levels.push(level);
    }

    levels
}

fn get_max_grace_ms(targets: &[ResolvedService]) -> u64 {
    targets
        .iter()
        .filter_map(|t| t.def.healthcheck.as_ref()?.grace_ms)
        .max()
        .unwrap_or(0)
}

// ============================================================================
// SERVICE LIFECYCLE
// ============================================================================

pub async fn cmd_start(
    managers: &HashMap<String, SessionManager>,
    targets: &[ResolvedService],
    all_services: &[ResolvedService],
    detached: bool,
) -> Result<(), String> {
    log_system(&format!("Starting {} process(es)...", targets.len()));

    let levels = compute_startup_order(targets);

    // Track remediated checks across all services in this startup
    let mut remediated = std::collections::HashSet::new();

    for (i, level) in levels.iter().enumerate() {
        for target in level {
            let mgr = managers.get(&target.group).unwrap();
            mgr.start(&target.name, &target.def, &mut remediated)
                .await?;
        }

        // Wait for grace period before next level
        if i < levels.len() - 1 {
            let grace_ms = get_max_grace_ms(level);
            if grace_ms > 0 {
                log_verbose(&format!(
                    "Waiting {}ms grace period for: {}",
                    grace_ms,
                    level
                        .iter()
                        .map(|t| t.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;
            }
        }
    }

    if detached {
        log_system("Processes started in background");
        return Ok(());
    }

    // Monitor mode
    monitor(managers, targets, all_services).await;
    Ok(())
}

async fn monitor(
    managers: &HashMap<String, SessionManager>,
    targets: &[ResolvedService],
    all_services: &[ResolvedService],
) {
    log_system("Monitoring processes... (Ctrl+C to stop all)");

    let log_stream = stream_logs(managers, targets, all_services, false);

    let targets_owned: Vec<ResolvedService> = targets.to_vec();
    let managers_clone: HashMap<String, SessionManager> = managers.clone();

    // Monitor for dead services + Ctrl+C
    let mut dead_services = std::collections::HashSet::new();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            log_stream.cleanup();
            log_system("Received SIGINT, stopping all processes...");
            cmd_stop(&managers_clone, &targets_owned).await;
        }
        _ = async {
            loop {
                for target in &targets_owned {
                    if dead_services.contains(&target.name) {
                        continue;
                    }
                    let mgr = managers_clone.get(&target.group).unwrap();
                    let status = mgr.status(&target.name).await;
                    if !status.running && status.exit_code.is_some() {
                        log(&format!("Exited with code {:?}", status.exit_code), &target.name, "red");
                        dead_services.insert(target.name.clone());
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        } => {}
    }
}

pub async fn cmd_stop(managers: &HashMap<String, SessionManager>, targets: &[ResolvedService]) {
    let mut stopped_any = false;

    for target in targets {
        let mgr = managers.get(&target.group).unwrap();
        if mgr.exists(&target.name).await {
            mgr.stop(&target.name).await;
            stopped_any = true;
        }
    }

    if stopped_any {
        log_system("All processes stopped");
    } else {
        log_system("No processes were running");
    }
}

pub async fn cmd_kill(managers: &HashMap<String, SessionManager>, targets: &[ResolvedService]) {
    let mut killed_any = false;

    for target in targets {
        let mgr = managers.get(&target.group).unwrap();
        if mgr.exists(&target.name).await {
            mgr.kill_service(&target.name).await;
            killed_any = true;
        }
    }

    if killed_any {
        log_system("All processes killed");
    } else {
        log_system("No processes were running");
    }
}

pub async fn cmd_restart(
    managers: &HashMap<String, SessionManager>,
    targets: &[ResolvedService],
) -> Result<(), String> {
    log_system(&format!("Restarting {} process(es)...", targets.len()));

    let mut remediated = std::collections::HashSet::new();
    for target in targets {
        let mgr = managers.get(&target.group).unwrap();
        mgr.stop(&target.name).await;
        mgr.start(&target.name, &target.def, &mut remediated)
            .await?;
    }
    Ok(())
}

// ============================================================================
// OBSERVABILITY
// ============================================================================

pub async fn cmd_ps(
    managers: &HashMap<String, SessionManager>,
    targets: &[ResolvedService],
    full: bool,
) {
    let mut sessions_by_service = HashMap::new();
    for (_group, mgr) in managers {
        let sessions = mgr.list_all().await;
        for session in sessions {
            sessions_by_service.insert(session.name.clone(), session);
        }
    }

    print("");

    if full {
        print(&format!(
            "{}GROUP          SERVICE        STATUS       MEM    CPU  PORTS            UPTIME      PID{}",
            c("bold"), c("reset")
        ));
        print(&"─".repeat(95));
    } else {
        print(&format!(
            "{}GROUP          SERVICE        STATUS       UPTIME{}",
            c("bold"),
            c("reset")
        ));
        print(&"─".repeat(55));
    }

    for target in targets {
        let session = sessions_by_service.get(&target.name);
        let (status_text, status_color, uptime) = match session {
            None => ("stopped".to_string(), "dim", "-".to_string()),
            Some(s) if s.running => {
                let uptime = if let Some(created) = s.created {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let elapsed = now.saturating_sub(created);
                    if elapsed >= 3600 {
                        format!("{}h {}m", elapsed / 3600, (elapsed % 3600) / 60)
                    } else if elapsed >= 60 {
                        format!("{}m {}s", elapsed / 60, elapsed % 60)
                    } else {
                        format!("{}s", elapsed)
                    }
                } else {
                    "-".to_string()
                };
                ("running".to_string(), "green", uptime)
            }
            Some(s) => (
                format!("exit({})", s.exit_code.unwrap_or(-1)),
                "red",
                "-".to_string(),
            ),
        };

        if full {
            let (pid, mem, cpu, ports) = if session.map(|s| s.running).unwrap_or(false) {
                if let Some(pid) = session.and_then(|s| s.pid) {
                    let metrics = get_process_metrics(pid).await;
                    (
                        pid.to_string(),
                        format!("{}M", metrics.memory_mb),
                        format!("{}%", metrics.cpu_percent),
                        if metrics.ports.is_empty() {
                            "-".to_string()
                        } else {
                            metrics
                                .ports
                                .iter()
                                .map(|p| p.to_string())
                                .collect::<Vec<_>>()
                                .join(",")
                        },
                    )
                } else {
                    (
                        "-".to_string(),
                        "-".to_string(),
                        "-".to_string(),
                        "-".to_string(),
                    )
                }
            } else {
                (
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                )
            };

            print(&format!(
                "{:<14} {:<14} {}{:<12}{} {:>5} {:>5}  {:<16} {:<10} {}",
                target.group,
                target.name,
                c(status_color),
                status_text,
                c("reset"),
                mem,
                cpu,
                ports,
                uptime,
                pid
            ));
        } else {
            print(&format!(
                "{:<14} {:<14} {}{:<12}{} {}",
                target.group,
                target.name,
                c(status_color),
                status_text,
                c("reset"),
                uptime
            ));
        }
    }
    print("");
}

pub async fn cmd_top(managers: &HashMap<String, SessionManager>, targets: &[ResolvedService]) {
    let mut stdout = std::io::stdout();

    // Enter raw mode + alternate screen for clean TUI
    if terminal::enable_raw_mode().is_err() {
        cmd_ps(managers, targets, true).await;
        return;
    }
    let _ = execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide);

    let result = top_loop(managers, targets, &mut stdout).await;

    // Always restore terminal, even on error
    let _ = execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();

    if let Err(e) = result {
        log_error(&e);
    }
}

/// Per-service cached metrics with last-update timestamp
struct ServiceMetrics {
    memory_mb: u64,
    cpu_percent: f64,
    ports: Vec<u16>,
    last_update: std::time::Instant,
}

impl ServiceMetrics {
    fn stale() -> Self {
        Self {
            memory_mb: 0,
            cpu_percent: 0.0,
            ports: Vec::new(),
            last_update: std::time::Instant::now() - std::time::Duration::from_secs(60),
        }
    }

    /// Higher CPU -> refresh more often
    fn refresh_interval(&self) -> std::time::Duration {
        if self.cpu_percent > 10.0 {
            std::time::Duration::from_secs(1)
        } else if self.cpu_percent > 1.0 {
            std::time::Duration::from_secs(2)
        } else {
            std::time::Duration::from_secs(5)
        }
    }
}

fn format_started(created: u64) -> String {
    use chrono::TimeZone;
    if let Some(dt) = chrono::Local.timestamp_opt(created as i64, 0).single() {
        dt.format("%H:%M:%S").to_string()
    } else {
        "-".to_string()
    }
}

async fn top_loop(
    managers: &HashMap<String, SessionManager>,
    targets: &[ResolvedService],
    stdout: &mut std::io::Stdout,
) -> Result<(), String> {
    use crate::process::SessionStatus;

    // Cached state per service
    let mut metrics_cache: HashMap<String, ServiceMetrics> = HashMap::new();
    let mut sessions_cache: HashMap<String, SessionStatus> = HashMap::new();

    for target in targets {
        metrics_cache.insert(target.name.clone(), ServiceMetrics::stale());
    }

    loop {
        // 1. Refresh session status (cheap: single tmux call per group)
        for (_group, mgr) in managers {
            for session in mgr.list_all().await {
                sessions_cache.insert(session.name.clone(), session);
            }
        }

        // 2. Smart metric refresh: only update 2-3 stale services per tick
        let now = std::time::Instant::now();
        let mut updated = 0;
        for target in targets {
            if updated >= 3 {
                break;
            }

            let session = sessions_cache.get(&target.name);
            let is_running = session.map(|s| s.running).unwrap_or(false);
            let pid = session.and_then(|s| s.pid);

            if is_running {
                if let Some(pid) = pid {
                    let cached = metrics_cache.get(&target.name).unwrap();
                    if now.duration_since(cached.last_update) >= cached.refresh_interval() {
                        let m = get_process_metrics(pid).await;
                        metrics_cache.insert(
                            target.name.clone(),
                            ServiceMetrics {
                                memory_mb: m.memory_mb,
                                cpu_percent: m.cpu_percent,
                                ports: m.ports,
                                last_update: now,
                            },
                        );
                        updated += 1;
                    }
                }
            } else {
                // Reset metrics for stopped services
                metrics_cache.insert(target.name.clone(), ServiceMetrics::stale());
            }
        }

        // 3. Render entire frame into a buffer, then flush once (no flicker)
        let mut buf = String::with_capacity(2048);
        let ts = chrono::Local::now().format("%H:%M:%S");

        // Move cursor home (no clear — we overwrite in place)
        buf.push_str("\x1b[H");

        buf.push_str(&format!(
            "{}rig top{} — {} — {} service(s)  {}(q to quit){}\r\n\r\n",
            c_raw("bold"),
            c_raw("reset"),
            ts,
            targets.len(),
            c_raw("dim"),
            c_raw("reset"),
        ));

        buf.push_str(&format!(
            "{}GROUP          SERVICE        STATUS       MEM    CPU    PORTS            STARTED{}\r\n",
            c_raw("bold"), c_raw("reset"),
        ));
        buf.push_str(&"─".repeat(87));
        buf.push_str("\r\n");

        for target in targets {
            let session = sessions_cache.get(&target.name);
            let m = metrics_cache.get(&target.name).unwrap();

            let (status_str, mem, cpu, ports, started) = match session {
                None
                | Some(SessionStatus {
                    running: false,
                    exit_code: None,
                    ..
                }) => (
                    format!("{}{:<12}{}", c_raw("dim"), "stopped", c_raw("reset")),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                ),
                Some(s) if !s.running => (
                    format!(
                        "{}{:<12}{}",
                        c_raw("red"),
                        format!("exit({})", s.exit_code.unwrap_or(-1)),
                        c_raw("reset")
                    ),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    s.created
                        .map(format_started)
                        .unwrap_or_else(|| "-".to_string()),
                ),
                Some(s) => {
                    let cpu_color = if m.cpu_percent > 50.0 {
                        "red"
                    } else if m.cpu_percent > 10.0 {
                        "yellow"
                    } else {
                        "reset"
                    };
                    let port_str = if m.ports.is_empty() {
                        "-".to_string()
                    } else {
                        let joined = m
                            .ports
                            .iter()
                            .take(3)
                            .map(|p| p.to_string())
                            .collect::<Vec<_>>()
                            .join(",");
                        if m.ports.len() > 3 {
                            format!("{}...", joined)
                        } else {
                            joined
                        }
                    };
                    (
                        format!("{}{:<12}{}", c_raw("green"), "running", c_raw("reset")),
                        format!("{}M", m.memory_mb),
                        format!(
                            "{}{:.1}%{}",
                            c_raw(cpu_color),
                            m.cpu_percent,
                            c_raw("reset")
                        ),
                        port_str,
                        s.created
                            .map(format_started)
                            .unwrap_or_else(|| "-".to_string()),
                    )
                }
            };

            // Pad status_str accounting for ANSI codes (raw length differs from visible)
            buf.push_str(&format!(
                "{:<14} {:<14} {} {:>5} {:>6}  {:<16} {}\r\n",
                target.group, target.name, status_str, mem, cpu, ports, started,
            ));
        }

        // Clear any leftover lines from a previous wider render
        buf.push_str("\x1b[J");

        let _ = write!(stdout, "{}", buf);
        let _ = stdout.flush();

        // 4. Wait 500ms or until keypress
        let poll_timeout = std::time::Duration::from_millis(500);
        if event::poll(poll_timeout).unwrap_or(false) {
            if let Ok(event::Event::Key(key)) = event::read() {
                match key.code {
                    event::KeyCode::Char('q') | event::KeyCode::Char('Q') => break,
                    event::KeyCode::Char('c')
                        if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                    {
                        break
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// Returns Err with a user-facing message if there are no logs.
pub async fn cmd_logs(
    managers: &HashMap<String, SessionManager>,
    targets: &[ResolvedService],
    all_services: &[ResolvedService],
    follow: bool,
    previous: bool,
) -> Result<(), String> {
    // Check if any log files exist
    let mut any_logs = false;
    for target in targets {
        let mgr = managers.get(&target.group).unwrap();
        let log_file = mgr.log_file(&target.name, previous);
        if let Ok(metadata) = tokio::fs::metadata(&log_file).await {
            if metadata.len() > 0 {
                any_logs = true;
                break;
            }
        }
    }

    if !any_logs && previous {
        return Err("No previous logs found".to_string());
    }

    if !any_logs && !follow {
        return if targets.len() == 1 {
            Err(format!("No logs for '{}'", targets[0].name))
        } else {
            Err("No logs found".to_string())
        };
    }

    if follow && previous {
        return Err("Cannot follow previous logs".to_string());
    }

    if follow {
        let log_stream = stream_logs(managers, targets, all_services, false);
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log_stream.cleanup();
            }
        }
    } else {
        // Dump mode
        for target in targets {
            let mgr = managers.get(&target.group).unwrap();
            let log_file = mgr.log_file(&target.name, previous);
            let color = get_service_color(all_services, &target.name);
            if let Ok(content) = tokio::fs::read_to_string(&log_file).await {
                for line in content.lines() {
                    let clean = strip_control_codes(line);
                    if !clean.trim().is_empty() {
                        log(&clean, &target.name, &color);
                    }
                }
            }
        }
    }

    Ok(())
}

// ============================================================================
// TASKS
// ============================================================================

/// Orchestrate running one or more resolved tasks through the provider.
///
/// Single task runs directly (args allowed). Multiple tasks reject args, then
/// run sequentially fail-fast, or in parallel (continue on failure, return the
/// first non-zero exit code). `run` is synchronous, so the parallel path uses
/// scoped threads.
pub fn cmd_tasks(
    provider: &(dyn TaskProvider + Send + Sync),
    tasks: &[ResolvedTask],
    args: &[String],
    parallel: bool,
) -> i32 {
    // Single task
    if tasks.len() == 1 {
        return provider.run(&tasks[0], args);
    }

    // Multiple tasks with args is an error
    if !args.is_empty() {
        log_error("Cannot pass arguments when running multiple tasks");
        return 1;
    }

    if !parallel {
        // Sequential (fail-fast)
        for task in tasks {
            let code = provider.run(task, &[]);
            if code != 0 {
                log_error(&format!(
                    "Task '{}' failed with exit code {}",
                    task.path, code
                ));
                return code;
            }
        }
        0
    } else {
        // Parallel: scoped threads borrow the shared provider and tasks.
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = tasks
                .iter()
                .map(|task| {
                    scope.spawn(move || {
                        let code = provider.run(task, &[]);
                        if code != 0 {
                            log_error(&format!(
                                "Task '{}' failed with exit code {}",
                                task.path, code
                            ));
                        }
                        (task.path.clone(), code)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });

        let mut first_error = 0;
        for (_path, code) in results {
            if code != 0 && first_error == 0 {
                first_error = code;
            }
        }
        first_error
    }
}

/// Returns Err if no tasks found, Ok(()) otherwise.
pub fn cmd_task_list(tasks: &[ResolvedTask], group_filter: Option<&str>) -> Result<(), String> {
    let filtered: Vec<_> = if let Some(group) = group_filter {
        tasks
            .iter()
            .filter(|t| crate::config::group_matches(&t.group, group))
            .cloned()
            .collect()
    } else {
        tasks.to_vec()
    };

    if filtered.is_empty() {
        return if let Some(group) = group_filter {
            Err(format!("No tasks found in group '{}'", group))
        } else {
            Err("No tasks defined in config".to_string())
        };
    }

    print("");
    for task in &filtered {
        let max_cmd_len = 50;
        let cmd = if task.command.len() > max_cmd_len {
            format!("{}...", &task.command[..max_cmd_len - 3])
        } else {
            task.command.clone()
        };
        let desc = if let Some(d) = &task.description {
            format!(" {}{}{}", c("dim"), d, c("reset"))
        } else {
            String::new()
        };
        // Default-goal marker (make tasks only), aligned with a 2-col prefix.
        let marker = if task.default_goal { "→ " } else { "  " };
        print(&format!(
            "{}{}{:<30}{} {}{}",
            marker,
            c("cyan"),
            task.path,
            c("reset"),
            cmd,
            desc
        ));
    }
    print("");
    Ok(())
}

// ============================================================================
// CONFIG DISPLAY
// ============================================================================

pub fn cmd_config(root: &Group, targets: &[ResolvedService], raw: bool, json: bool) {
    // Machine-readable output mirrors the actual loaded group tree (nested
    // services + tasks, Makefile-sourced tasks included), independent of the
    // service/group filter used for the human-readable listing.
    if raw || json {
        let output = group_tree_to_value(root);
        if json {
            print(&serde_json::to_string_pretty(&output).unwrap_or_default());
        } else {
            print(&serde_yaml::to_string(&output).unwrap_or_default());
        }
        return;
    }

    if targets.is_empty() {
        log_error("No matching services found");
        return;
    }

    let cwd = std::env::current_dir().unwrap_or_default();
    let cwd_str = cwd.to_string_lossy();

    for target in targets {
        let color = get_service_color(&[target.clone()], &target.name);
        let wd = make_relative(&target.def.working_dir, &cwd_str);
        print(&format!(
            "{}{:<12}{} {}{:<12}{} {} {}working_dir={}{}",
            c("dim"),
            target.group,
            c("reset"),
            c(&color),
            target.name,
            c("reset"),
            target.def.command,
            c("dim"),
            wd,
            c("reset")
        ));
    }
}

fn make_relative(path: &str, cwd: &str) -> String {
    if path.starts_with(cwd) {
        let rest = path[cwd.len()..].trim_start_matches('/');
        if rest.is_empty() {
            ".".to_string()
        } else {
            rest.to_string()
        }
    } else {
        path.to_string()
    }
}

pub async fn cmd_init() -> Result<(), String> {
    for name in crate::config::CONFIG_NAMES {
        if std::path::Path::new(name).exists() {
            return Err(format!("{} already exists", name));
        }
    }

    let template = "\
# rig config. You usually don't need this: a folder with a bare Makefile already
# works zero-config — `rig tasks` lists its targets and `rig run <target>` runs
# them. Add this file only for services (long-running processes) or richer tasks.
#
# This folder is the project root, so names below are bare (no group prefix).
# Subfolders holding a Makefile or rig file auto-become dotted child groups.

services:
  api:
    command: npm start
    working_dir: .
    environment:
      PORT: \"3000\"

tasks:
  build:
    command: npm run build
    description: Build the project   # working_dir defaults to this folder
";

    std::fs::write("rig.yaml", template)
        .map_err(|e| format!("Failed to create rig.yaml: {}", e))?;

    log_system("Created rig.yaml");
    Ok(())
}

// ============================================================================
// DISCOVERY
// ============================================================================

/// Walk `root_dir` downward, gitignore-aware, returning every file path. Shared
/// ignore setup (hidden dirs like `.git` skipped at any depth, `.gitignore` honored
/// even outside a git repo, vendored/build dirs excluded) reused by both rig-file and
/// Makefile discovery.
pub(crate) fn walk_ignored_files(root_dir: &str) -> Result<Vec<std::path::PathBuf>, String> {
    let mut builder = ignore::WalkBuilder::new(root_dir);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false);

    let mut overrides = ignore::overrides::OverrideBuilder::new(root_dir);
    for dir in [
        "node_modules",
        ".git",
        "vendor",
        ".rig",
        "__pycache__",
        ".venv",
        "dist",
        "build",
    ] {
        overrides
            .add(&format!("!{}/**", dir))
            .map_err(|e| e.to_string())?;
    }
    builder.overrides(overrides.build().map_err(|e| e.to_string())?);

    let mut results = Vec::new();
    for entry in builder.build().flatten() {
        if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            results.push(entry.path().to_path_buf());
        }
    }
    Ok(results)
}
