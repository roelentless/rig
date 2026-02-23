/**
 * Terminal output: colors, logging, and display helpers.
 */

// Color palette
export const COLORS: Record<string, string> = {
  cyan: "\x1b[36m",
  yellow: "\x1b[33m",
  magenta: "\x1b[35m",
  green: "\x1b[32m",
  blue: "\x1b[34m",
  orange: "\x1b[38;5;208m",
  red: "\x1b[31m",
  lavender: "\x1b[38;5;183m",
  pink: "\x1b[38;5;213m",
  teal: "\x1b[38;5;51m",
  lime: "\x1b[38;5;154m",
  coral: "\x1b[38;5;209m",
  sky: "\x1b[38;5;117m",
  gold: "\x1b[38;5;220m",
  violet: "\x1b[38;5;135m",
  reset: "\x1b[0m",
  dim: "\x1b[2m",
  bold: "\x1b[1m",
};

// Deterministic color palette for services (assigned by index)
// Order matters: avoid red/yellow early (error/warning associations)
// First ~10 should be distinct, non-alarming colors
export const SERVICE_COLORS = [
  "cyan",
  "yellow",
  "magenta",
  "teal",
  "green",
  "blue",
  "orange",
  "pink",
  "lavender",
  "violet",
  "lime",
  "coral",
  "sky",
  "gold",
  "violet",
];

// Disable colors when not a TTY (piping to other commands)
export const IS_TTY = Deno.stdout.isTerminal();

// ASCII key codes for cmdTop
export const KEY_Q_LOWER = 113;
export const KEY_Q_UPPER = 81;
export const KEY_CTRL_C = 3;

// Global verbose flag
let VERBOSE = false;

export function setVerbose(v: boolean): void {
  VERBOSE = v;
}

// Get color code only if TTY
export function c(color: string): string {
  if (!IS_TTY) return "";
  return COLORS[color] ?? "";
}

// Safe print that handles broken pipe (EPIPE) gracefully
export function print(msg: string): void {
  try {
    console.log(msg);
  } catch (e) {
    if (e instanceof Deno.errors.BrokenPipe) {
      Deno.exit(0);
    }
    throw e;
  }
}

export function log(msg: string, prefix?: string, color?: string): void {
  const now = new Date();
  const ts =
    now.toLocaleTimeString("en-US", {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }) +
    "." +
    now.getMilliseconds().toString().padStart(3, "0");

  const colorCode = color ? c(color) : "";
  const prefixStr = prefix
    ? `${colorCode}${prefix.padEnd(12)}${c("reset")} `
    : "";

  print(`${c("dim")}${ts}${c("reset")} ${prefixStr}${msg}`);
}

export function logSystem(msg: string): void {
  log(msg, "rig", "bold");
}

export function logError(msg: string): void {
  // Log errors to stderr
  const now = new Date();
  const ts =
    now.getHours().toString().padStart(2, "0") +
    ":" +
    now.getMinutes().toString().padStart(2, "0") +
    ":" +
    now.getSeconds().toString().padStart(2, "0") +
    "." +
    now.getMilliseconds().toString().padStart(3, "0");
  console.error(`${c("dim")}${ts}${c("reset")} ${c("red")}${"rig".padEnd(12)}${c("reset")} ${msg}`);
}

export function logVerbose(msg: string): void {
  if (VERBOSE) {
    log(msg, "rig", "dim");
  }
}

/**
 * Strip ANSI control codes that would mess up log prefixes.
 * Preserves color codes but removes cursor movement, line clearing, etc.
 */
export function stripControlCodes(line: string): string {
  return line
    .replace(/\r/g, "")                     // Carriage return
    .replace(/\x1b\[\d*[ABCD]/g, "")        // Cursor movement (up/down/forward/back)
    .replace(/\x1b\[\d*;\d*[Hf]/g, "")      // Cursor position
    .replace(/\x1b\[\d*G/g, "")             // Cursor to column
    .replace(/\x1b\[\d*[JK]/g, "")          // Clear screen/line
    .replace(/\x1b\[\?25[lh]/g, "");        // Hide/show cursor
}
