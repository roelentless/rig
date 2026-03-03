use std::process;

use clap::{Parser, Subcommand};

use rig::commands::*;
use rig::config::*;
use rig::output::*;
use rig::process::*;

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
  tasks [--group <name>]           List all tasks
  run/task <task...> [-- args...]  Run task(s) (group.name or group.service.name)
    -p, --parallel                 Run tasks in parallel

MULTI-FILE:
  discover [--dry-run] [--yes] [path]  Scan for rig files and update imports

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
  rig run backend.deploy    Run a task
  rig run backend.api.test -- --coverage  Pass args to task
  rig run api.test web.test Run multiple tasks sequentially
  rig run api.test web.test -p  Run tasks in parallel
  rig config --json         Show raw JSON config
  rig discover              Scan for rig files and update imports
  rig discover --dry-run    Show what would be imported

CONFIG:
  Searches upward from current directory for rig.yaml, rig.yml, or *.rig.yaml.
  Supports imports to compose configs from multiple files:

    imports:
      - db/rig.yaml
      - backend/rig.yaml

  All imported files are merged into a flat namespace. Service names must be
  unique across all files. Circular imports are detected and reported.
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

    // Discovery
    Discover {
        path: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(short = 'y', long)]
        yes: bool,
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
            let (config, _) = load_config(None).unwrap_or_else(|e| handle_config_error(e));
            let group_filter = group.first().map(|s| s.as_str());
            if let Err(e) = cmd_task_list(&config, group_filter) {
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
            let (config, _) = load_config(None).unwrap_or_else(|e| handle_config_error(e));

            if list {
                let group_filter = group.first().map(|s| s.as_str());
                if let Err(e) = cmd_task_list(&config, group_filter) {
                    handle_error(&e);
                }
                return;
            }

            if tasks.is_empty() {
                let group_filter = group.first().map(|s| s.as_str());
                if let Err(e) = cmd_task_list(&config, group_filter) {
                    handle_error(&e);
                }
                return;
            }

            let resolved: Vec<ResolvedTask> = tasks
                .iter()
                .map(|p| resolve_task(p, &config).unwrap_or_else(|e| handle_config_error(e)))
                .collect();

            let code = cmd_tasks(&resolved, &pass_args, parallel).await;
            process::exit(code);
        }

        // Discovery
        Commands::Discover { path, dry_run, yes } => {
            let dir = path.unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            if let Err(e) = cmd_discover(&dir, dry_run, yes).await {
                handle_error(&e);
            }
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
                 -> Result<(Config, String, Vec<ResolvedService>), ConfigError> {
                    let (config, config_dir) = load_config(None)?;
                    let lookup = build_service_lookup(&config);
                    let targets = resolve_targets(&config, &lookup, services, groups)?;
                    Ok((config, config_dir, targets))
                };

            match command {
                Commands::Start { services, d, group } | Commands::Up { services, d, group } => {
                    let services = expand_csv(&services);
                    let (config, config_dir, targets) = load_and_resolve(&services, &group)
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
                    let (_config, config_dir, targets) = load_and_resolve(&services, &group)
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
                    let all_services = get_all_services(&config);
                    cmd_config(&config, &targets, &all_services, raw, json);
                }
                _ => unreachable!(),
            }
        }
    }
}
