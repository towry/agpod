import {
  existsSync,
  readFileSync,
  writeFileSync,
  renameSync,
  mkdirSync,
  openSync,
  closeSync,
  rmSync,
  statSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { execFileSync } from "node:child_process";

export interface HiveAgentRecord {
  agent_id: string;
  worker_name: string;
  agent_kind: "codex" | "claude";
  model?: string | null;
  workdir: string;
  window_id: string;
  window_name: string;
  pane_id: string;
  status: "spawning" | "idle" | "busy" | "resetting" | "dead";
  last_used_at_ms?: number | null;
}

export interface HiveSessionRecord {
  version: number;
  session_id: string;
  session_name: string;
  queen_pane_id: string;
  tmux_socket?: string | null;
  repo_root: string;
  agent_limit: number;
  updated_at_ms: number;
  agents: HiveAgentRecord[];
}

export interface HiveModeContext {
  stateDir: string;
  sessionId: string;
  agentId: string;
  queenPaneId: string;
  workerPaneId: string;
  workerName?: string;
}

const HIVE_LOCK_STALE_MS = 30_000;

export function resolveCcHooksPath(...parts: string[]): string {
  const base =
    process.env.CC_HOOKS_SRC
    || (process.env.HOME ? join(process.env.HOME, ".dotfiles", "packages", "cc-hooks", "src") : "");
  if (!base) {
    throw new Error("CC_HOOKS_SRC or HOME is required to resolve cc-hooks runtime path");
  }
  return resolve(base, ...parts);
}

export function detectHiveMode(): HiveModeContext | null {
  const stateDir = process.env.AGPOD_HIVE_STATE_DIR?.trim();
  const sessionId = process.env.AGPOD_HIVE_SESSION_ID?.trim();
  const agentId = process.env.AGPOD_HIVE_AGENT_ID?.trim();
  const queenPaneId = process.env.TMUX_HIVE_QUEEN?.trim();
  const workerPaneId = process.env.TMUX_PANE?.trim();

  if (!stateDir || !sessionId || !agentId || !queenPaneId || !workerPaneId) {
    return null;
  }
  return {
    stateDir,
    sessionId,
    agentId,
    queenPaneId,
    workerPaneId,
    workerName: process.env.TMUX_HIVE_WORKER_NAME?.trim() || undefined,
  };
}

export function sessionFilePath(ctx: HiveModeContext): string {
  return join(ctx.stateDir, `${ctx.sessionId}.json`);
}

export function sessionLockPath(ctx: HiveModeContext): string {
  return join(ctx.stateDir, `${ctx.sessionId}.lock`);
}

export function loadSession(ctx: HiveModeContext): HiveSessionRecord | null {
  const file = sessionFilePath(ctx);
  if (!existsSync(file)) return null;
  try {
    return JSON.parse(readFileSync(file, "utf8")) as HiveSessionRecord;
  } catch {
    return null;
  }
}

export function validateHiveMode(ctx: HiveModeContext): HiveSessionRecord | null {
  const session = loadSession(ctx);
  if (!session) return null;
  if (session.session_id !== ctx.sessionId) return null;
  if (session.queen_pane_id !== ctx.queenPaneId) return null;
  const agent = session.agents.find((item) => item.agent_id === ctx.agentId);
  if (!agent) return null;
  if (agent.pane_id !== ctx.workerPaneId) return null;
  return session;
}

export function updateAgentStatus(
  session: HiveSessionRecord,
  ctx: HiveModeContext,
  status: HiveAgentRecord["status"],
): HiveSessionRecord {
  const now = Date.now();
  session.updated_at_ms = now;
  session.agents = session.agents.map((agent) =>
    agent.agent_id === ctx.agentId
      ? {
          ...agent,
          status,
          last_used_at_ms: now,
          worker_name: ctx.workerName ?? agent.worker_name,
        }
      : agent,
  );
  return session;
}

export function saveSession(ctx: HiveModeContext, session: HiveSessionRecord): void {
  const file = sessionFilePath(ctx);
  mkdirSync(dirname(file), { recursive: true });
  const tmp = `${file}.tmp`;
  writeFileSync(tmp, JSON.stringify(session, null, 2));
  renameSync(tmp, file);
}

export async function withHiveLock<T>(ctx: HiveModeContext, run: () => T | Promise<T>): Promise<T> {
  mkdirSync(ctx.stateDir, { recursive: true });
  const lock = sessionLockPath(ctx);
  let fd = -1;
  for (let attempt = 0; attempt < 200; attempt += 1) {
    try {
      fd = openSync(lock, "wx");
      break;
    } catch (error) {
      if (!(error instanceof Error) || !String((error as NodeJS.ErrnoException).code).includes("EEXIST")) {
        throw error;
      }
      if (isLockStale(lock, HIVE_LOCK_STALE_MS)) {
        rmSync(lock, { force: true });
        continue;
      }
      execFileSync("sleep", ["0.025"]);
    }
  }
  if (fd < 0) {
    throw new Error(`timed out waiting for hive lock: ${lock}`);
  }
  try {
    return await run();
  } finally {
    closeSync(fd);
    rmSync(lock, { force: true });
  }
}

function isLockStale(lockPath: string, staleAfterMs: number): boolean {
  try {
    const stat = statSync(lockPath);
    return Date.now() - stat.mtimeMs >= staleAfterMs;
  } catch {
    return false;
  }
}

export function queenPaneExists(queenPaneId: string): boolean {
  try {
    const output = execFileSync("tmux", ["list-panes", "-a", "-F", "#{pane_id}"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    });
    return output.split("\n").includes(queenPaneId);
  } catch {
    return false;
  }
}

export function sendKeysToPane(targetPaneId: string, text: string): void {
  execFileSync("tmux", ["send-keys", "-t", targetPaneId, "C-u"]);
  execFileSync("sleep", ["0.3"]);
  execFileSync("tmux", ["send-keys", "-t", targetPaneId, "-l", text]);
  execFileSync("sleep", ["0.3"]);
  execFileSync("tmux", ["send-keys", "-t", targetPaneId, "C-m"]);
}
