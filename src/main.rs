use std::process;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use rig::commands::*;
use rig::config::*;
use rig::output::*;
use rig::process::*;
use rig::providers::{provider, TaskProvider};

const VERSION: &str = env!("CARGO_PKG_VERSION");

// Custom help text matching the tool's interface
const HELP_TEXT: &str = r#"
rig - lightweight dev workflow tool for services and tasks

USAGE:
  rig <command> [options] [services...]

SERVICES:
  init                      Create rig.yaml in current directory
  start/up [services...]    Start processes (foreground, streaming logs)
  start/up -d [services...] Start processes in background (detached)
  stop/down [services...]   Stop processes (graceful)
  kill [services...]        Force kill with SIGKILL
  restart [services...]     Restart processes
  ps/list [-f|--full]       Show status (add -f for mem/cpu/ports)
  top                       Live dashboard with auto-refreshing metrics
  logs/tail [-f] [--prev] [service]  Show logs (--prev for last run)
  config [--raw|--json] [services...] Show config (--raw for YAML, --json for JSON)

TASKS:
  tasks [--group <name>]           List all tasks (→ marks a Makefile default goal)
  run/task <task...> [-- args...]  Run task(s): name, group.name, or group.service.name
    -p, --parallel                 Run tasks in parallel
    -l, --list                     List tasks instead of running them

OTHER:
  version                   Show version
  help                      Show this help

OPTIONS:
  -g, --group <name>        Target entire group(s) instead of services
  -v, --verbose             Enable verbose logging for debugging

EXAMPLES:
  rig up                    Start all processes (all groups)
  rig up -d                 Start all in background
  rig start api worker      Start specific services
  rig start api,worker      Same, comma-separated
  rig start -g backend      Start all services in backend group
  rig down                  Stop all processes (graceful)
  rig stop -g backend       Stop all services in backend group
  rig kill                  Force kill all processes
  rig restart -g backend    Restart entire group
  rig ps                    Show status
  rig logs -f               Follow all logs
  rig logs --prev api       Show previous logs for api
  rig tasks                 List all tasks
  rig run build             Run a task by name (Makefile target or rig task)
  rig run backend.deploy    Run a namespaced task
  rig run backend.api.test -- --coverage  Pass args to task
  rig run api.test web.test Run multiple tasks sequentially
  rig run api.test web.test -p  Run tasks in parallel
  rig config --json         Show raw JSON config

CONFIG:
  Zero-config: a folder with a Makefile just works — `rig tasks` lists its
  targets and `rig run <target>` runs them via make. Root Makefile targets are
  bare names, ./sub/Makefile targets become sub.<target>, nested folders dot
  deeper. Each Makefile's default goal is marked with → in listings.

  For services (and richer tasks) add rig.yaml, rig.yml, or *.rig.yaml. Config
  is a folder-aware group tree: rig searches upward for the nearest directory
  holding a rig file or Makefile (the project root) and builds the tree downward
  from there (gitignore-aware). Multiple rig files in one directory compose at
  the same level. Top-level tasks/services/environment/env_file need no
  wrapper (root = CWD → bare names). Any subfolder holding a Makefile or rig
  file auto-becomes a child group named by its folder. Authored `groups:`
  reshape the tree: a group is backed by `dir:` (adopt/rename a folder with its
  Makefile and/or rig file), `paths:` (explicit rig files), and/or inline
  units and child groups. `environment` and `env_file` cascade ancestor-wins —
  a higher group wraps a project and injects env from above; env files load at
  run/start, not at list time.
"#;

#[derive(Parser)]
#[command(name = "rig", version = VERSION, disable_help_flag = true, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Show help
    #[arg(short = 'h', long = "help", global = true)]
    help: bool,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    Help,
    Version,
    Init,

    // Service commands
    Start {
        services: Vec<String>,
        #[arg(short, long)]
        d: bool,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Up {
        services: Vec<String>,
        #[arg(short, long)]
        d: bool,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Stop {
        services: Vec<String>,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Down {
        services: Vec<String>,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Kill {
        services: Vec<String>,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Restart {
        services: Vec<String>,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Ps {
        #[arg(short = 'f', long)]
        full: bool,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    List {
        #[arg(short = 'f', long)]
        full: bool,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Top {
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Logs {
        services: Vec<String>,
        #[arg(short, long)]
        f: bool,
        #[arg(long)]
        prev: bool,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Tail {
        services: Vec<String>,
        #[arg(short, long)]
        f: bool,
        #[arg(long)]
        prev: bool,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Config {
        services: Vec<String>,
        #[arg(long)]
        raw: bool,
        #[arg(long)]
        json: bool,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },

    // Task commands
    Tasks {
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
    },
    Run {
        tasks: Vec<String>,
        #[arg(short = 'p', long)]
        parallel: bool,
        #[arg(short = 'l', long)]
        list: bool,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
        #[arg(last = true)]
        pass_args: Vec<String>,
    },
    Task {
        tasks: Vec<String>,
        #[arg(short = 'p', long)]
        parallel: bool,
        #[arg(short = 'l', long)]
        list: bool,
        #[arg(short, long, num_args = 1)]
        group: Vec<String>,
        #[arg(last = true)]
        pass_args: Vec<String>,
    },
}

/// Split comma-separated service names: ["api,worker", "db"] -> ["api", "worker", "db"]
fn expand_csv(names: &[String]) -> Vec<String> {
    names
        .iter()
        .flat_map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        })
        .collect()
}

fn handle_config_error(e: ConfigError) -> ! {
    log_error(&e.to_string());
    process::exit(1);
}

fn handle_error(msg: &str) -> ! {
    log_error(msg);
    process::exit(1);
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    set_verbose(cli.verbose);

    if cli.help {
        print(HELP_TEXT);
        return;
    }

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            print(HELP_TEXT);
            return;
        }
    };

    match command {
        Commands::Help => {
            print(HELP_TEXT);
        }
        Commands::Version => {
            print(VERSION);
        }
        Commands::Init => {
            if let Err(e) = cmd_init().await {
                handle_error(&e);
            }
        }

        // Task listing
        Commands::Tasks { group } => {
            let p = provider(None).unwrap_or_else(|e| handle_config_error(e));
            let all = p.discover();
            let group_filter = group.first().map(|s| s.as_str());
            if let Err(e) = cmd_task_list(&all, group_filter) {
                handle_error(&e);
            }
        }

        // Task running
        Commands::Run {
            tasks,
            parallel,
            list,
            group,
            pass_args,
        }
        | Commands::Task {
            tasks,
            parallel,
            list,
            group,
            pass_args,
        } => {
            let p = provider(None).unwrap_or_else(|e| handle_config_error(e));

            if list || tasks.is_empty() {
                let all = p.discover();
                let group_filter = group.first().map(|s| s.as_str());
                if let Err(e) = cmd_task_list(&all, group_filter) {
                    handle_error(&e);
                }
                return;
            }

            let resolved: Vec<ResolvedTask> = tasks
                .iter()
                .map(|path| p.resolve(path).unwrap_or_else(|e| handle_config_error(e)))
                .collect();

            let code = run_tasks_supervised(p, resolved, pass_args, parallel).await;
            process::exit(code);
        }

        // Service commands — need tmux
        _ => {
            if !check_tmux().await {
                print_tmux_install_guide();
                process::exit(1);
            }

            // Load config for service commands
            let load_and_resolve =
                |services: &[String],
                 groups: &[String]|
                 -> Result<(Group, String, Vec<ResolvedService>), ConfigError> {
                    let (config, config_dir) = load_config(None)?;
                    let lookup = build_service_lookup(&config);
                    let targets = resolve_targets(&config, &lookup, services, groups)?;
                    Ok((config, config_dir, targets))
                };

            match command {
                Commands::Start { services, d, group } | Commands::Up { services, d, group } => {
                    let services = expand_csv(&services);
                    let (config, config_dir, mut targets) = load_and_resolve(&services, &group)
                        .unwrap_or_else(|e| handle_config_error(e));
                    // Load env files now (start time), fail-fast on required-missing.
                    materialize_targets_env(&config, &mut targets)
                        .unwrap_or_else(|e| handle_config_error(e));
                    let all_services = get_all_services(&config);
                    let managers = create_managers(&targets, &config_dir);
                    if let Err(e) = cmd_start(&managers, &targets, &all_services, d).await {
                        handle_error(&e);
                    }
                }
                Commands::Stop { services, group } | Commands::Down { services, group } => {
                    let services = expand_csv(&services);
                    let (_config, config_dir, targets) = load_and_resolve(&services, &group)
                        .unwrap_or_else(|e| handle_config_error(e));
                    let managers = create_managers(&targets, &config_dir);
                    cmd_stop(&managers, &targets).await;
                }
                Commands::Kill { services, group } => {
                    let services = expand_csv(&services);
                    let (_config, config_dir, targets) = load_and_resolve(&services, &group)
                        .unwrap_or_else(|e| handle_config_error(e));
                    let managers = create_managers(&targets, &config_dir);
                    cmd_kill(&managers, &targets).await;
                }
                Commands::Restart { services, group } => {
                    let services = expand_csv(&services);
                    let (config, config_dir, mut targets) = load_and_resolve(&services, &group)
                        .unwrap_or_else(|e| handle_config_error(e));
                    // Restart re-launches the service, so materialize env now too.
                    materialize_targets_env(&config, &mut targets)
                        .unwrap_or_else(|e| handle_config_error(e));
                    let managers = create_managers(&targets, &config_dir);
                    if let Err(e) = cmd_restart(&managers, &targets).await {
                        handle_error(&e);
                    }
                }
                Commands::Ps { full, group } | Commands::List { full, group } => {
                    let (_config, config_dir, targets) =
                        load_and_resolve(&[], &group).unwrap_or_else(|e| handle_config_error(e));
                    let managers = create_managers(&targets, &config_dir);
                    cmd_ps(&managers, &targets, full).await;
                }
                Commands::Top { group } => {
                    let (_config, config_dir, targets) =
                        load_and_resolve(&[], &group).unwrap_or_else(|e| handle_config_error(e));
                    let managers = create_managers(&targets, &config_dir);
                    cmd_top(&managers, &targets).await;
                }
                Commands::Logs {
                    services,
                    f,
                    prev,
                    group,
                }
                | Commands::Tail {
                    services,
                    f,
                    prev,
                    group,
                } => {
                    let services = expand_csv(&services);
                    let (config, config_dir, targets) = if services.is_empty() {
                        load_and_resolve(&[], &group).unwrap_or_else(|e| handle_config_error(e))
                    } else {
                        load_and_resolve(&services, &group)
                            .unwrap_or_else(|e| handle_config_error(e))
                    };
                    let all_services = get_all_services(&config);
                    let managers = create_managers(&targets, &config_dir);
                    if let Err(e) = cmd_logs(&managers, &targets, &all_services, f, prev).await {
                        handle_error(&e);
                    }
                }
                Commands::Config {
                    services,
                    raw,
                    json,
                    group,
                } => {
                    let services = expand_csv(&services);
                    let (config, _config_dir, targets) = if services.is_empty() {
                        load_and_resolve(&[], &group).unwrap_or_else(|e| handle_config_error(e))
                    } else {
                        load_and_resolve(&services, &group)
                            .unwrap_or_else(|e| handle_config_error(e))
                    };
                    cmd_config(&config, &targets, raw, json);
                }
                _ => unreachable!(),
            }
        }
    }
}

/// Run resolved tasks while forwarding an interrupt (SIGINT/SIGTERM) delivered to
/// rig down into the spawned task subtree, so a signalled `rig run` never leaves
/// the `sh -c`/`make`/recipe process tree behind as orphans. The task work runs on
/// a blocking thread and is raced against the two signals; on a signal the
/// provider cancels its task tree (graceful signal first, SIGKILL backstop —
/// providers own their tasks' cancellation semantics) and rig exits with the
/// conventional 128+signo code.
///
/// This wraps only the task-run path — the tmux/service commands are untouched.
async fn run_tasks_supervised(
    provider: Arc<dyn TaskProvider + Send + Sync>,
    resolved: Vec<ResolvedTask>,
    pass_args: Vec<String>,
    parallel: bool,
) -> i32 {
    use sysinfo::Signal;
    use tokio::signal::unix::{signal, SignalKind};

    let worker = provider.clone();
    let work = tokio::task::spawn_blocking(move || {
        cmd_tasks(worker.as_ref(), &resolved, &pass_args, parallel)
    });

    // If we cannot install signal handlers, fall back to plain waiting.
    let (mut sigint, mut sigterm) = match (
        signal(SignalKind::interrupt()),
        signal(SignalKind::terminate()),
    ) {
        (Ok(i), Ok(t)) => (i, t),
        _ => return work.await.unwrap_or(1),
    };

    tokio::select! {
        r = work => r.unwrap_or(1),
        _ = sigint.recv() => { provider.cancel(Signal::Interrupt); 130 }
        _ = sigterm.recv() => { provider.cancel(Signal::Term); 143 }
    }
}
