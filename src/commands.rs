use std::collections::HashMap;
use std::io::Write;
use std::process::Stdio;

use crossterm::{cursor, event, execute, terminal};
use tokio::process::Command;

use crate::config::{get_all_tasks, Config, ResolvedService, ResolvedTask};
use crate::output::{
    c, c_raw, log, log_error, log_system, log_verbose, print, strip_control_codes,
};
use crate::process::{get_process_metrics, get_service_color, stream_logs, SessionManager};

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

fn shell_escape(arg: &str) -> String {
    if arg
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '=' | '@' | ':'))
    {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

pub async fn run_task(resolved: &ResolvedTask, args: &[String]) -> i32 {
    let full_command = if args.is_empty() {
        resolved.command.clone()
    } else {
        format!(
            "{} {}",
            resolved.command,
            args.iter()
                .map(|a| shell_escape(a))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };

    log_verbose(&format!("task={}", resolved.path));
    log_verbose(&format!("command={}", full_command));
    log_verbose(&format!("working_dir={}", resolved.working_dir));

    let mut cmd = Command::new("sh");
    cmd.args(["-c", &full_command]);
    cmd.current_dir(&resolved.working_dir);
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    // Merge environment
    if let Some(env) = &resolved.environment {
        cmd.envs(env);
    }

    match cmd.status().await {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            log_error(&format!(
                "Failed to run task '{}' (command='{}', dir='{}'): {}",
                resolved.path, resolved.command, resolved.working_dir, e
            ));
            1
        }
    }
}

pub async fn cmd_tasks(tasks: &[ResolvedTask], args: &[String], parallel: bool) -> i32 {
    // Single task
    if tasks.len() == 1 {
        return run_task(&tasks[0], args).await;
    }

    // Multiple tasks with args is an error
    if !args.is_empty() {
        log_error("Cannot pass arguments when running multiple tasks");
        return 1;
    }

    if !parallel {
        // Sequential (fail-fast)
        for task in tasks {
            let code = run_task(task, &[]).await;
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
        // Parallel
        let mut handles = Vec::new();
        for task in tasks {
            let task = task.clone();
            handles.push(tokio::spawn(async move {
                let code = run_task(&task, &[]).await;
                if code != 0 {
                    log_error(&format!(
                        "Task '{}' failed with exit code {}",
                        task.path, code
                    ));
                }
                (task.path.clone(), code)
            }));
        }

        let mut first_error = 0;
        for handle in handles {
            if let Ok((_path, code)) = handle.await {
                if code != 0 && first_error == 0 {
                    first_error = code;
                }
            }
        }
        first_error
    }
}

/// Returns Err if no tasks found, Ok(()) otherwise.
pub fn cmd_task_list(config: &Config, group_filter: Option<&str>) -> Result<(), String> {
    let tasks = get_all_tasks(config);
    let filtered: Vec<_> = if let Some(group) = group_filter {
        tasks.into_iter().filter(|t| t.group == group).collect()
    } else {
        tasks
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
        print(&format!(
            "{}{:<30}{} {}{}",
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

pub fn cmd_config(
    _config: &Config,
    targets: &[ResolvedService],
    _all_services: &[ResolvedService],
    raw: bool,
    json: bool,
) {
    if targets.is_empty() {
        log_error("No matching services found");
        return;
    }

    if raw || json {
        // Build output structure
        let mut output = serde_json::Map::new();
        let mut groups_map = serde_json::Map::new();

        for target in targets {
            let group_entry = groups_map
                .entry(target.group.clone())
                .or_insert_with(|| serde_json::json!({"services": {}}));
            if let Some(services) = group_entry.get_mut("services") {
                if let Some(obj) = services.as_object_mut() {
                    obj.insert(
                        target.name.clone(),
                        serde_json::to_value(&target.def).unwrap_or_default(),
                    );
                }
            }
        }

        output.insert("groups".to_string(), serde_json::Value::Object(groups_map));

        if json {
            print(&serde_json::to_string_pretty(&output).unwrap_or_default());
        } else {
            print(&serde_yaml::to_string(&output).unwrap_or_default());
        }
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

    let template = "groups:\n  myapp:\n    services:\n      api:\n        command: npm start\n        working_dir: ./somewhere\n        environment:\n          PORT: \"3000\"\n";

    std::fs::write("rig.yaml", template)
        .map_err(|e| format!("Failed to create rig.yaml: {}", e))?;

    log_system("Created rig.yaml");
    Ok(())
}

// ============================================================================
// DISCOVERY
// ============================================================================

fn scan_for_rig_files(root_dir: &str) -> Result<Vec<String>, String> {
    let pattern = regex::Regex::new(r"(^rig\.ya?ml$|.*\.rig\.yaml$)").unwrap();

    let mut builder = ignore::WalkBuilder::new(root_dir);
    builder
        .hidden(false)
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
            let name = entry.file_name().to_string_lossy();
            if pattern.is_match(&name) {
                results.push(entry.path().to_string_lossy().to_string());
            }
        }
    }
    Ok(results)
}

pub async fn cmd_discover(root_dir: &str, dry_run: bool, auto_accept: bool) -> Result<(), String> {
    let abs_root = if root_dir.starts_with('/') {
        root_dir.to_string()
    } else {
        let cwd = std::env::current_dir().unwrap_or_default();
        format!("{}/{}", cwd.display(), root_dir)
    };

    print(&format!("Scanning from {}...\n", abs_root));

    let mut all_files = scan_for_rig_files(&abs_root)?;
    if all_files.is_empty() {
        print("No rig files found.");
        return Ok(());
    }

    all_files.sort();

    // Find root config
    let root_configs: Vec<_> = all_files
        .iter()
        .filter(|f| {
            let dir = std::path::Path::new(f)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            dir == abs_root || dir == abs_root.trim_end_matches('/')
        })
        .cloned()
        .collect();

    if root_configs.is_empty() {
        print(&format!("No root config found in {}", abs_root));
        print("\nFound rig files:");
        for f in &all_files {
            let rel = f.strip_prefix(&format!("{}/", abs_root)).unwrap_or(f);
            print(&format!("  {}", rel));
        }
        return Ok(());
    }

    let root_config = root_configs
        .iter()
        .find(|f| f.ends_with("/rig.yaml"))
        .unwrap_or(&root_configs[0])
        .clone();

    let root_config_rel = root_config
        .strip_prefix(&format!("{}/", abs_root))
        .unwrap_or(&root_config)
        .to_string();

    print("Found rig files:");
    for f in &all_files {
        let rel = f.strip_prefix(&format!("{}/", abs_root)).unwrap_or(f);
        let is_root = *f == root_config;
        print(&format!(
            "  {}{}",
            rel,
            if is_root { " (root)" } else { "" }
        ));
    }

    // Get current imports
    let current_imports: Vec<String> = if let Ok(content) = std::fs::read_to_string(&root_config) {
        if let Ok(raw) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
            raw.get("imports")
                .and_then(|v| v.as_sequence())
                .map(|seq| {
                    seq.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    print(&format!("\nCurrent imports in {}:", root_config_rel));
    if current_imports.is_empty() {
        print("  (none)");
    } else {
        for imp in &current_imports {
            print(&format!("  - {}", imp));
        }
    }

    // Find missing imports
    let imported_set: std::collections::HashSet<String> = current_imports
        .iter()
        .map(|imp| {
            if imp.starts_with('/') {
                imp.clone()
            } else {
                format!("{}/{}", abs_root, imp)
            }
        })
        .collect();

    let mut missing = Vec::new();
    for f in &all_files {
        if *f == root_config {
            continue;
        }
        if !imported_set.contains(f) {
            let rel = f.strip_prefix(&format!("{}/", abs_root)).unwrap_or(f);
            missing.push(rel.to_string());
        }
    }

    if missing.is_empty() {
        print("\nAll rig files are imported. Nothing to do.");
        return Ok(());
    }

    print("\nMissing (not imported):");
    for m in &missing {
        print(&format!("  + {}", m));
    }

    if dry_run {
        print("\n[Dry run] Would add the above imports to rig.yaml");
        return Ok(());
    }

    if !auto_accept {
        print("\nUse --yes to auto-accept changes.");
        return Ok(());
    }

    // Update config
    let content = std::fs::read_to_string(&root_config)
        .map_err(|e| format!("Failed to read {}: {}", root_config, e))?;
    let mut raw: serde_yaml::Value = serde_yaml::from_str(&content)
        .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));

    let mapping = raw
        .as_mapping_mut()
        .ok_or_else(|| "Root config is not a YAML mapping".to_string())?;
    let imports_key = serde_yaml::Value::String("imports".to_string());
    let existing_imports: Vec<String> = mapping
        .get(&imports_key)
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut new_imports: Vec<serde_yaml::Value> = existing_imports
        .iter()
        .map(|s| serde_yaml::Value::String(s.clone()))
        .collect();
    for m in &missing {
        new_imports.push(serde_yaml::Value::String(m.clone()));
    }

    mapping.insert(imports_key, serde_yaml::Value::Sequence(new_imports));

    let new_content =
        serde_yaml::to_string(&raw).map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(&root_config, &new_content)
        .map_err(|e| format!("Failed to write {}: {}", root_config, e))?;

    print(&format!(
        "\nUpdated {} with {} new import(s).",
        root_config_rel,
        missing.len()
    ));
    Ok(())
}
