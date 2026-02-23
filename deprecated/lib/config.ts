/**
 * Configuration: types, schema validation, loading, parsing, and querying.
 */

// JSR imports required for global install - deno install resolves JSR packages correctly
// deno-lint-ignore no-import-prefix
import { parse as parseYaml } from "jsr:@std/yaml@^1.0.11";
// deno-lint-ignore no-import-prefix
import { parse as parseEnv } from "jsr:@std/dotenv@^0.225";

import { logVerbose } from "./output.ts";

// ============================================================================
// TYPES
// ============================================================================

export interface HealthCheck {
  grace_ms?: number;
}

export interface EnvFileEntry {
  path: string;
  required?: boolean;  // defaults to true
}

export interface WatchDef {
  paths?: string[];       // Directories/files to watch (relative to working_dir)
  extensions?: string[];  // File extensions (e.g., ["ts", "tsx"])
  patterns?: string[];    // Include glob patterns (watchexec --filter)
  ignore?: string[];      // Exclude glob patterns (watchexec --ignore)
  debounce?: string;      // Debounce duration (e.g., "500ms")
}

export interface TaskDef {
  command: string;
  working_dir?: string;                    // Required for group tasks, inherited for service tasks
  environment?: Record<string, string>;
  env_file?: string | EnvFileEntry[];
  description?: string;
}

export interface RequirementDef {
  check: string;
  command: string;
}

export interface ServiceDef {
  command: string;
  working_dir: string;
  environment?: Record<string, string>;
  env_file?: string | EnvFileEntry[];
  color?: string;
  depends_on?: string[];
  healthcheck?: HealthCheck;
  tasks?: Record<string, TaskDef>;         // Service-level tasks
  watch?: WatchDef;                        // Auto-restart via watchexec
  requirements?: RequirementDef[];         // Pre-start checks with remediation
}

export interface GroupDef {
  services?: Record<string, ServiceDef>;   // Optional - group can have only tasks
  tasks?: Record<string, TaskDef>;         // Group-level tasks
}

export interface Config {
  groups: Record<string, GroupDef>;
}

// Raw config as parsed from YAML (before processing)
export interface RawConfig {
  imports?: string[];
  groups?: Record<string, unknown>;
}

// Context for recursive config loading
interface LoadContext {
  configPath: string;      // absolute path to this rig file
  configDir: string;       // dirname (execution context for this file)
  loaded: Set<string>;     // already-imported files (dedup by absolute path)
  importChain: string[];   // for circular import detection
}

// Resolved task with merged config from hierarchy
export interface ResolvedTask {
  path: string;           // e.g., "backend.api.build" or "backend.deploy"
  group: string;
  service?: string;       // undefined for group-level tasks
  name: string;           // task name
  command: string;
  working_dir: string;
  environment?: Record<string, string>;
  description?: string;
}

// Flattened service for easy lookup
export interface ResolvedService {
  group: string;
  name: string;
  def: ServiceDef;
}

// ============================================================================
// CONSTANTS
// ============================================================================

export const CONFIG_NAMES = ["rig.yaml", "rig.yml"];
export const CONFIG_PATTERN = /^(rig\.ya?ml|.*\.rig\.yaml)$/;  // matches rig.yaml, rig.yml, *.rig.yaml
export const LOG_DIR = ".rig/logs";

// ============================================================================
// ERRORS
// ============================================================================

/** Config errors are displayed cleanly without stack traces */
export class ConfigError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ConfigError";
  }
}

// ============================================================================
// SCHEMA VALIDATION
// ============================================================================

// Known keys at each level of the config hierarchy
const SCHEMA = {
  root: new Set(["imports", "groups"]),
  group: new Set(["services", "tasks"]),
  service: new Set(["command", "working_dir", "environment", "env_file", "color", "depends_on", "healthcheck", "tasks", "watch", "requirements"]),
  task: new Set(["command", "working_dir", "environment", "env_file", "description"]),
  watch: new Set(["paths", "extensions", "patterns", "ignore", "debounce"]),
  healthcheck: new Set(["grace_ms"]),
  envFileEntry: new Set(["path", "required"]),
  requirement: new Set(["check", "command"]),
};

/**
 * Validate that an object only contains expected keys.
 * Returns an array of error messages for unknown keys.
 */
function validateKeys(
  obj: Record<string, unknown>,
  allowedKeys: Set<string>,
  context: string,
  configPath: string
): string[] {
  const errors: string[] = [];
  for (const key of Object.keys(obj)) {
    if (!allowedKeys.has(key)) {
      const allowed = Array.from(allowedKeys).sort().join(", ");
      errors.push(`Unknown key '${key}' in ${context} (${configPath}). Valid keys: ${allowed}`);
    }
  }
  return errors;
}

/**
 * Validate the entire config structure, checking for unknown keys at all levels.
 */
function validateConfigSchema(raw: Record<string, unknown>, configPath: string): void {
  const errors: string[] = [];

  // Validate root level
  errors.push(...validateKeys(raw, SCHEMA.root, "config root", configPath));

  // Validate groups
  if (raw.groups && typeof raw.groups === "object") {
    for (const [groupName, groupDef] of Object.entries(raw.groups as Record<string, unknown>)) {
      if (!groupDef || typeof groupDef !== "object") continue;
      const g = groupDef as Record<string, unknown>;

      errors.push(...validateKeys(g, SCHEMA.group, `group '${groupName}'`, configPath));

      // Validate services within group
      if (g.services && typeof g.services === "object") {
        for (const [serviceName, serviceDef] of Object.entries(g.services as Record<string, unknown>)) {
          if (!serviceDef || typeof serviceDef !== "object") continue;
          const s = serviceDef as Record<string, unknown>;

          errors.push(...validateKeys(s, SCHEMA.service, `service '${groupName}.${serviceName}'`, configPath));

          // Validate watch config
          if (s.watch && typeof s.watch === "object") {
            errors.push(...validateKeys(s.watch as Record<string, unknown>, SCHEMA.watch, `watch in service '${groupName}.${serviceName}'`, configPath));
          }

          // Validate healthcheck config
          if (s.healthcheck && typeof s.healthcheck === "object") {
            errors.push(...validateKeys(s.healthcheck as Record<string, unknown>, SCHEMA.healthcheck, `healthcheck in service '${groupName}.${serviceName}'`, configPath));
          }

          // Validate env_file entries if array of objects
          if (s.env_file && Array.isArray(s.env_file)) {
            for (const entry of s.env_file) {
              if (entry && typeof entry === "object") {
                errors.push(...validateKeys(entry as Record<string, unknown>, SCHEMA.envFileEntry, `env_file entry in service '${groupName}.${serviceName}'`, configPath));
              }
            }
          }

          // Validate requirements entries
          if (s.requirements && Array.isArray(s.requirements)) {
            for (const entry of s.requirements) {
              if (entry && typeof entry === "object") {
                errors.push(...validateKeys(entry as Record<string, unknown>, SCHEMA.requirement, `requirement in service '${groupName}.${serviceName}'`, configPath));
              }
            }
          }

          // Validate service-level tasks
          if (s.tasks && typeof s.tasks === "object") {
            for (const [taskName, taskDef] of Object.entries(s.tasks as Record<string, unknown>)) {
              if (!taskDef || typeof taskDef !== "object") continue;
              errors.push(...validateKeys(taskDef as Record<string, unknown>, SCHEMA.task, `task '${groupName}.${serviceName}.${taskName}'`, configPath));

              // Validate env_file entries in task
              const t = taskDef as Record<string, unknown>;
              if (t.env_file && Array.isArray(t.env_file)) {
                for (const entry of t.env_file) {
                  if (entry && typeof entry === "object") {
                    errors.push(...validateKeys(entry as Record<string, unknown>, SCHEMA.envFileEntry, `env_file entry in task '${groupName}.${serviceName}.${taskName}'`, configPath));
                  }
                }
              }
            }
          }
        }
      }

      // Validate group-level tasks
      if (g.tasks && typeof g.tasks === "object") {
        for (const [taskName, taskDef] of Object.entries(g.tasks as Record<string, unknown>)) {
          if (!taskDef || typeof taskDef !== "object") continue;
          errors.push(...validateKeys(taskDef as Record<string, unknown>, SCHEMA.task, `task '${groupName}.${taskName}'`, configPath));

          // Validate env_file entries in group task
          const t = taskDef as Record<string, unknown>;
          if (t.env_file && Array.isArray(t.env_file)) {
            for (const entry of t.env_file) {
              if (entry && typeof entry === "object") {
                errors.push(...validateKeys(entry as Record<string, unknown>, SCHEMA.envFileEntry, `env_file entry in task '${groupName}.${taskName}'`, configPath));
              }
            }
          }
        }
      }
    }
  }

  // If any errors, throw them all at once
  if (errors.length > 0) {
    throw new ConfigError(`Config validation failed:\n  ${errors.join("\n  ")}`);
  }
}

// ============================================================================
// PATH & ENV HELPERS
// ============================================================================

/**
 * Normalize a path by resolving . and .. components.
 * Uses URL to handle path normalization.
 */
export function normalizePath(path: string): string {
  const normalized = new URL(path, "file:///").pathname;
  // Remove trailing slash (except for root "/")
  return normalized.length > 1 && normalized.endsWith("/")
    ? normalized.slice(0, -1)
    : normalized;
}

/**
 * Normalize a path for env_file entries, resolving relative to configDir.
 */
function normalizeEnvFilePath(envFile: string, configDir: string): string {
  if (envFile.startsWith("/")) {
    return envFile;
  }
  return `${configDir}/${envFile}`;
}

async function loadEnvFiles(
  entries: EnvFileEntry[],
  configDir: string
): Promise<Record<string, string>> {
  const result: Record<string, string> = {};

  for (const entry of entries) {
    // Resolve path relative to config dir
    const fullPath = entry.path.startsWith("/")
      ? entry.path
      : `${configDir}/${entry.path}`;

    try {
      const content = await Deno.readTextFile(fullPath);
      const parsed = parseEnv(content);
      Object.assign(result, parsed);
    } catch (err) {
      if (entry.required !== false) {
        throw new ConfigError(`Failed to load env file '${entry.path}': ${err instanceof Error ? err.message : String(err)}`);
      }
      // required: false - silently skip
    }
  }

  return result;
}

// ============================================================================
// CONFIG LOADING
// ============================================================================

/**
 * Walk upward from startDir to find the nearest rig config file.
 * Checks for rig.yaml, rig.yml first, then *.rig.yaml files.
 * Returns absolute path to the config file.
 */
export async function findNearestConfig(startDir: string): Promise<string> {
  // Resolve to absolute path
  let dir = startDir.startsWith("/") ? startDir : await Deno.realPath(startDir);

  while (true) {
    // Check for standard names first (rig.yaml, rig.yml)
    for (const name of CONFIG_NAMES) {
      const path = `${dir}/${name}`;
      try {
        await Deno.stat(path);
        return path;
      } catch {
        // Continue
      }
    }

    // Check for *.rig.yaml files
    try {
      for await (const entry of Deno.readDir(dir)) {
        if (entry.isFile && entry.name.endsWith(".rig.yaml") && entry.name !== "rig.yaml") {
          return `${dir}/${entry.name}`;
        }
      }
    } catch {
      // Directory not readable, continue to parent
    }

    // Move to parent directory
    const parent = dir.replace(/\/[^/]+$/, "") || "/";
    if (parent === dir) {
      // Reached filesystem root
      throw new ConfigError("No rig config found (searched up to filesystem root). Expected: rig.yaml, rig.yml, or *.rig.yaml");
    }
    dir = parent;
  }
}

/**
 * Legacy function for backward compatibility - finds config in current directory only.
 */
export async function findConfig(): Promise<string> {
  for (const name of CONFIG_NAMES) {
    try {
      await Deno.stat(name);
      return name;
    } catch {
      // Continue
    }
  }
  throw new ConfigError(`Config file not found. Expected: ${CONFIG_NAMES.join(" or ")}`);
}

/**
 * Parse a single rig config file and expand paths relative to its directory.
 * Does not process imports - returns raw parsed content with expanded paths.
 */
export async function parseConfigFile(configPath: string): Promise<{ raw: RawConfig; configDir: string }> {
  let content: string;
  try {
    content = await Deno.readTextFile(configPath);
  } catch (err) {
    throw new ConfigError(`Failed to read config file '${configPath}': ${err instanceof Error ? err.message : String(err)}`);
  }

  let raw: RawConfig;
  try {
    raw = parseYaml(content) as RawConfig;
  } catch (err) {
    throw new ConfigError(`Invalid YAML in ${configPath}: ${err instanceof Error ? err.message : String(err)}`);
  }

  // Handle empty file or null content
  if (!raw) {
    raw = {};
  }

  // Validate schema before returning
  validateConfigSchema(raw as Record<string, unknown>, configPath);

  const configDir = configPath.replace(/\/[^/]+$/, "");
  return { raw, configDir };
}

/**
 * Recursively load a config tree, processing imports and merging groups.
 * Each file's paths are expanded relative to its own location before merging.
 */
async function loadConfigRecursive(ctx: LoadContext): Promise<{ groups: Record<string, GroupDef>; seenServices: Map<string, string> }> {
  const { configPath, configDir, loaded, importChain } = ctx;

  // Circular import check
  if (importChain.includes(configPath)) {
    const cycle = [...importChain, configPath].map(p => p.replace(Deno.cwd() + "/", "./")).join("\n  → ");
    throw new ConfigError(`Circular import detected:\n  ${cycle}`);
  }

  // Dedup check - already loaded this exact file
  if (loaded.has(configPath)) {
    return { groups: {}, seenServices: new Map() };
  }
  loaded.add(configPath);

  // Parse the file
  const { raw } = await parseConfigFile(configPath);

  // Track groups and services from this file
  const groups: Record<string, GroupDef> = {};
  const seenServices = new Map<string, string>(); // serviceName -> groupName

  // Process groups from this file
  if (raw.groups && typeof raw.groups === "object") {
    for (const [groupName, groupDef] of Object.entries(raw.groups as Record<string, unknown>)) {
      // Validate group name
      if (!/^[a-zA-Z0-9_-]+$/.test(groupName)) {
        throw new ConfigError(`Invalid group name '${groupName}' in ${configPath}: must be alphanumeric with hyphens/underscores only`);
      }

      const g = groupDef as Record<string, unknown>;

      // Groups can have services, tasks, or both
      if (!g.services && !g.tasks) {
        throw new ConfigError(`Group '${groupName}' in ${configPath} must have 'services' and/or 'tasks'`);
      }

      const services: Record<string, ServiceDef> = {};
      const groupTasks: Record<string, TaskDef> = {};

      // Parse services
      if (g.services && typeof g.services === "object") {
        for (const [name, def] of Object.entries(g.services as Record<string, unknown>)) {
          const d = def as Record<string, unknown>;
          if (!d.command || typeof d.command !== "string") {
            throw new ConfigError(`Service '${groupName}.${name}' in ${configPath} must have a 'command' field`);
          }
          if (!d.working_dir || typeof d.working_dir !== "string") {
            throw new ConfigError(`Service '${groupName}.${name}' in ${configPath} must have a 'working_dir' field`);
          }

          // Resolve relative working_dir paths relative to this config's directory
          let working_dir = d.working_dir as string;
          if (!working_dir.startsWith("/")) {
            working_dir = `${configDir}/${working_dir}`;
          }

          // Convert all environment values to strings
          const environment = d.environment
            ? Object.fromEntries(
                Object.entries(d.environment as Record<string, unknown>).map(([k, v]) => [k, String(v)])
              )
            : undefined;

          // Parse depends_on
          let depends_on: string[] | undefined;
          if (d.depends_on) {
            if (!Array.isArray(d.depends_on)) {
              throw new ConfigError(`Service '${groupName}.${name}' in ${configPath} depends_on must be an array`);
            }
            depends_on = d.depends_on as string[];
          }

          // Parse healthcheck
          let healthcheck: HealthCheck | undefined;
          if (d.healthcheck) {
            const hc = d.healthcheck as Record<string, unknown>;
            healthcheck = {};
            if (hc.grace_ms !== undefined) {
              healthcheck.grace_ms = Number(hc.grace_ms);
            }
          }

          // Parse env_file - normalize paths relative to this config's directory
          let envFileEntries: EnvFileEntry[] = [];
          if (d.env_file) {
            if (typeof d.env_file === "string") {
              envFileEntries = [{ path: normalizeEnvFilePath(d.env_file, configDir), required: true }];
            } else if (Array.isArray(d.env_file)) {
              envFileEntries = (d.env_file as unknown[]).map((entry) => {
                if (typeof entry === "string") {
                  return { path: normalizeEnvFilePath(entry, configDir), required: true };
                }
                const e = entry as Record<string, unknown>;
                return {
                  path: normalizeEnvFilePath(e.path as string, configDir),
                  required: e.required !== false,
                };
              });
            }
          }

          // Load env files and merge (inline environment overrides env_file)
          let mergedEnvironment = environment;
          if (envFileEntries.length > 0) {
            const envFromFiles = await loadEnvFiles(envFileEntries, "/"); // paths already absolute
            mergedEnvironment = { ...envFromFiles, ...environment };
          }

          // Parse service-level tasks
          let serviceTasks: Record<string, TaskDef> | undefined;
          if (d.tasks && typeof d.tasks === "object") {
            serviceTasks = {};
            for (const [taskName, taskDef] of Object.entries(d.tasks as Record<string, unknown>)) {
              const r = taskDef as Record<string, unknown>;
              if (!r.command || typeof r.command !== "string") {
                throw new ConfigError(`Task '${groupName}.${name}.${taskName}' in ${configPath} must have a 'command' field`);
              }

              // Resolve working_dir if specified
              let taskWorkingDir = r.working_dir as string | undefined;
              if (taskWorkingDir && !taskWorkingDir.startsWith("/")) {
                taskWorkingDir = `${configDir}/${taskWorkingDir}`;
              }

              // Parse env_file for service task
              let taskEnvFileEntries: EnvFileEntry[] = [];
              if (r.env_file) {
                if (typeof r.env_file === "string") {
                  taskEnvFileEntries = [{ path: normalizeEnvFilePath(r.env_file, configDir), required: true }];
                } else if (Array.isArray(r.env_file)) {
                  taskEnvFileEntries = (r.env_file as unknown[]).map((entry) => {
                    if (typeof entry === "string") {
                      return { path: normalizeEnvFilePath(entry, configDir), required: true };
                    }
                    const e = entry as Record<string, unknown>;
                    return {
                      path: normalizeEnvFilePath(e.path as string, configDir),
                      required: e.required !== false,
                    };
                  });
                }
              }

              // Load env files for service task
              let taskEnvironment = r.environment
                ? Object.fromEntries(
                    Object.entries(r.environment as Record<string, unknown>).map(([k, v]) => [k, String(v)])
                  )
                : undefined;
              if (taskEnvFileEntries.length > 0) {
                const envFromFiles = await loadEnvFiles(taskEnvFileEntries, "/");
                taskEnvironment = { ...envFromFiles, ...taskEnvironment };
              }

              serviceTasks[taskName] = {
                command: r.command as string,
                working_dir: taskWorkingDir,
                environment: taskEnvironment,
                description: r.description as string | undefined,
              };
            }
          }

          // Parse watch config
          let watch: WatchDef | undefined;
          if (d.watch && typeof d.watch === "object") {
            const w = d.watch as Record<string, unknown>;
            watch = {};

            // Parse paths - resolve relative to working_dir (already absolute)
            if (w.paths && Array.isArray(w.paths)) {
              watch.paths = (w.paths as string[]).map(p => {
                if (p.startsWith("/")) return normalizePath(p);
                return normalizePath(`${working_dir}/${p}`);
              });
            }

            // Parse extensions
            if (w.extensions && Array.isArray(w.extensions)) {
              watch.extensions = w.extensions as string[];
            }

            // Parse patterns (include filters)
            if (w.patterns && Array.isArray(w.patterns)) {
              watch.patterns = w.patterns as string[];
            }

            // Parse ignore patterns
            if (w.ignore && Array.isArray(w.ignore)) {
              watch.ignore = w.ignore as string[];
            }

            // Parse debounce
            if (w.debounce !== undefined) {
              watch.debounce = String(w.debounce);
            }
          }

          // Parse requirements
          let requirements: RequirementDef[] | undefined;
          if (d.requirements && Array.isArray(d.requirements)) {
            requirements = [];
            for (const entry of d.requirements as unknown[]) {
              if (!entry || typeof entry !== "object") {
                throw new ConfigError(`Invalid requirement entry in service '${groupName}.${name}' (${configPath}): must be an object with 'check' and 'command'`);
              }
              const r = entry as Record<string, unknown>;
              if (!r.check || typeof r.check !== "string") {
                throw new ConfigError(`Requirement in service '${groupName}.${name}' (${configPath}) must have a 'check' string`);
              }
              if (!r.command || typeof r.command !== "string") {
                throw new ConfigError(`Requirement in service '${groupName}.${name}' (${configPath}) must have a 'command' string`);
              }
              requirements.push({ check: r.check, command: r.command });
            }
          }

          services[name] = {
            command: d.command as string,
            working_dir,
            environment: mergedEnvironment,
            color: d.color as string | undefined,
            depends_on,
            healthcheck,
            tasks: serviceTasks,
            watch,
            requirements,
          };

          seenServices.set(name, groupName);
        }
      }

      // Parse group-level tasks
      if (g.tasks && typeof g.tasks === "object") {
        for (const [taskName, taskDef] of Object.entries(g.tasks as Record<string, unknown>)) {
          const r = taskDef as Record<string, unknown>;
          if (!r.command || typeof r.command !== "string") {
            throw new ConfigError(`Task '${groupName}.${taskName}' in ${configPath} must have a 'command' field`);
          }
          if (!r.working_dir || typeof r.working_dir !== "string") {
            throw new ConfigError(`Task '${groupName}.${taskName}' in ${configPath} must have a 'working_dir' field (group-level tasks cannot inherit)`);
          }

          // Resolve relative working_dir
          let taskWorkingDir = r.working_dir as string;
          if (!taskWorkingDir.startsWith("/")) {
            taskWorkingDir = `${configDir}/${taskWorkingDir}`;
          }

          // Parse env_file for group task
          let taskEnvFileEntries: EnvFileEntry[] = [];
          if (r.env_file) {
            if (typeof r.env_file === "string") {
              taskEnvFileEntries = [{ path: normalizeEnvFilePath(r.env_file, configDir), required: true }];
            } else if (Array.isArray(r.env_file)) {
              taskEnvFileEntries = (r.env_file as unknown[]).map((entry) => {
                if (typeof entry === "string") {
                  return { path: normalizeEnvFilePath(entry, configDir), required: true };
                }
                const e = entry as Record<string, unknown>;
                return {
                  path: normalizeEnvFilePath(e.path as string, configDir),
                  required: e.required !== false,
                };
              });
            }
          }

          // Load env files for group task
          let taskEnvironment = r.environment
            ? Object.fromEntries(
                Object.entries(r.environment as Record<string, unknown>).map(([k, v]) => [k, String(v)])
              )
            : undefined;
          if (taskEnvFileEntries.length > 0) {
            const envFromFiles = await loadEnvFiles(taskEnvFileEntries, "/");
            taskEnvironment = { ...envFromFiles, ...taskEnvironment };
          }

          groupTasks[taskName] = {
            command: r.command as string,
            working_dir: taskWorkingDir,
            environment: taskEnvironment,
            description: r.description as string | undefined,
          };
        }
      }

      groups[groupName] = {
        services: Object.keys(services).length > 0 ? services : undefined,
        tasks: Object.keys(groupTasks).length > 0 ? groupTasks : undefined,
      };
    }
  }

  // Process imports recursively
  if (raw.imports && Array.isArray(raw.imports)) {
    for (const importPath of raw.imports) {
      // Resolve import path relative to this config's directory and normalize
      let absImportPath = importPath.startsWith("/")
        ? importPath
        : `${configDir}/${importPath}`;

      // Normalize path (resolve .. and .)
      absImportPath = normalizePath(absImportPath);

      // Check if import exists
      try {
        await Deno.stat(absImportPath);
      } catch {
        throw new ConfigError(`Import not found: ${importPath}\n  in ${configPath}`);
      }

      const childCtx: LoadContext = {
        configPath: absImportPath,
        configDir: absImportPath.replace(/\/[^/]+$/, ""),
        loaded,                                    // shared dedup set
        importChain: [...importChain, configPath], // track for circular detection
      };

      const childResult = await loadConfigRecursive(childCtx);

      // Merge child groups into current groups, checking for duplicates
      for (const [groupName, groupDef] of Object.entries(childResult.groups)) {
        if (groups[groupName]) {
          throw new ConfigError(`Duplicate group '${groupName}' defined in:\n  - ${configPath}\n  - ${absImportPath}`);
        }
        groups[groupName] = groupDef;
      }

      // Merge child services, checking for duplicates
      for (const [serviceName, groupName] of childResult.seenServices) {
        if (seenServices.has(serviceName)) {
          throw new ConfigError(
            `Duplicate service '${serviceName}' found in:\n` +
            `  - group '${seenServices.get(serviceName)}'\n` +
            `  - group '${groupName}' in ${absImportPath}`
          );
        }
        seenServices.set(serviceName, groupName);
      }
    }
  }

  return { groups, seenServices };
}

/**
 * Load the full config tree starting from a root config file.
 */
async function loadConfigTree(rootPath: string): Promise<{ config: Config; configDir: string }> {
  // Normalize to absolute path
  const absPath = rootPath.startsWith("/") ? rootPath : `${Deno.cwd()}/${rootPath}`;
  const configDir = absPath.replace(/\/[^/]+$/, "");

  const ctx: LoadContext = {
    configPath: absPath,
    configDir,
    loaded: new Set(),
    importChain: [],
  };

  const result = await loadConfigRecursive(ctx);

  // Validate depends_on references exist across all groups
  for (const [groupName, groupDef] of Object.entries(result.groups)) {
    if (!groupDef.services) continue;
    for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
      for (const dep of serviceDef.depends_on ?? []) {
        if (!result.seenServices.has(dep)) {
          throw new ConfigError(`Service '${groupName}.${serviceName}' depends on unknown service '${dep}'`);
        }
      }
    }
  }

  return {
    config: { groups: result.groups },
    configDir,
  };
}

/**
 * Load config from a path (uses tree-based loading with import support).
 * If no path is provided, searches upward from CWD for nearest config.
 */
export async function loadConfig(configPath?: string): Promise<{ config: Config; configDir: string }> {
  const path = configPath ?? (await findNearestConfig(Deno.cwd()));
  logVerbose(`config=${path}`);
  return loadConfigTree(path);
}

// ============================================================================
// CONFIG QUERYING
// ============================================================================

/**
 * Build a flat lookup map from service name to its group and definition.
 * Since service names are unique across groups, this enables O(1) lookup.
 */
export function buildServiceLookup(config: Config): Map<string, ResolvedService> {
  const lookup = new Map<string, ResolvedService>();
  for (const [groupName, groupDef] of Object.entries(config.groups)) {
    if (!groupDef.services) continue;
    for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
      lookup.set(serviceName, { group: groupName, name: serviceName, def: serviceDef });
    }
  }
  return lookup;
}

/**
 * Get all services from config as a flat array.
 */
export function getAllServices(config: Config): ResolvedService[] {
  const services: ResolvedService[] = [];
  for (const [groupName, groupDef] of Object.entries(config.groups)) {
    if (!groupDef.services) continue;
    for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
      services.push({ group: groupName, name: serviceName, def: serviceDef });
    }
  }
  return services;
}

/**
 * Resolve a task path (e.g., "backend.api.build" or "backend.deploy") to a ResolvedTask.
 * For service-level tasks, merges service config with task config.
 */
export function resolveTask(path: string, config: Config): ResolvedTask {
  const parts = path.split(".");

  if (parts.length < 2 || parts.length > 3) {
    throw new ConfigError(`Invalid task path '${path}'. Use 'group.task' or 'group.service.task'`);
  }

  const [groupName, secondPart, thirdPart] = parts;

  const groupDef = config.groups[groupName];
  if (!groupDef) {
    throw new ConfigError(`Unknown group '${groupName}'`);
  }

  if (parts.length === 2) {
    // Could be group.task or group.service (with implicit task name?)
    // First try group-level task
    const taskName = secondPart;
    if (groupDef.tasks?.[taskName]) {
      const taskDef = groupDef.tasks[taskName];
      return {
        path,
        group: groupName,
        name: taskName,
        command: taskDef.command,
        working_dir: taskDef.working_dir!, // Required for group tasks
        environment: taskDef.environment,
        description: taskDef.description,
      };
    }

    // Not a group task - error
    throw new ConfigError(`Unknown task '${path}'. Did you mean 'group.service.task'?`);
  }

  // parts.length === 3: group.service.task
  const serviceName = secondPart;
  const taskName = thirdPart;

  const serviceDef = groupDef.services?.[serviceName];
  if (!serviceDef) {
    throw new ConfigError(`Unknown service '${groupName}.${serviceName}'`);
  }

  const taskDef = serviceDef.tasks?.[taskName];
  if (!taskDef) {
    throw new ConfigError(`Unknown task '${path}'`);
  }

  // Merge service config with task config
  // Order: service env -> task env (task overrides)
  const mergedEnv = taskDef.environment
    ? { ...serviceDef.environment, ...taskDef.environment }
    : serviceDef.environment;

  return {
    path,
    group: groupName,
    service: serviceName,
    name: taskName,
    command: taskDef.command,
    working_dir: taskDef.working_dir ?? serviceDef.working_dir, // Inherit from service
    environment: mergedEnv,
    description: taskDef.description,
  };
}

/**
 * Get all tasks from config as a flat array.
 */
export function getAllTasks(config: Config): ResolvedTask[] {
  const tasks: ResolvedTask[] = [];

  for (const [groupName, groupDef] of Object.entries(config.groups)) {
    // Group-level tasks
    if (groupDef.tasks) {
      for (const [taskName, taskDef] of Object.entries(groupDef.tasks)) {
        tasks.push({
          path: `${groupName}.${taskName}`,
          group: groupName,
          name: taskName,
          command: taskDef.command,
          working_dir: taskDef.working_dir!,
          environment: taskDef.environment,
          description: taskDef.description,
        });
      }
    }

    // Service-level tasks
    if (groupDef.services) {
      for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
        if (serviceDef.tasks) {
          for (const [taskName, taskDef] of Object.entries(serviceDef.tasks)) {
            const mergedEnv = taskDef.environment
              ? { ...serviceDef.environment, ...taskDef.environment }
              : serviceDef.environment;

            tasks.push({
              path: `${groupName}.${serviceName}.${taskName}`,
              group: groupName,
              service: serviceName,
              name: taskName,
              command: taskDef.command,
              working_dir: taskDef.working_dir ?? serviceDef.working_dir,
              environment: mergedEnv,
              description: taskDef.description,
            });
          }
        }
      }
    }
  }

  return tasks;
}

/**
 * Resolve CLI targets to a list of services.
 * - If groupNames provided: return all services in those groups
 * - If serviceNames provided: resolve each service name
 * - If neither: return all services
 */
export function resolveTargets(
  config: Config,
  lookup: Map<string, ResolvedService>,
  serviceNames: string[],
  groupNames: string[]
): ResolvedService[] {
  // If -g flags provided, get all services from those groups
  if (groupNames.length > 0) {
    const services: ResolvedService[] = [];
    for (const groupName of groupNames) {
      const groupDef = config.groups[groupName];
      if (!groupDef) {
        throw new ConfigError(`Unknown group: ${groupName}`);
      }
      if (!groupDef.services) continue;
      for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
        services.push({ group: groupName, name: serviceName, def: serviceDef });
      }
    }
    return services;
  }

  // If service names provided, resolve them
  if (serviceNames.length > 0) {
    const services: ResolvedService[] = [];
    for (const name of serviceNames) {
      const resolved = lookup.get(name);
      if (!resolved) {
        throw new ConfigError(`Unknown service: ${name}`);
      }
      services.push(resolved);
    }
    return services;
  }

  // No args - return all services
  return getAllServices(config);
}
