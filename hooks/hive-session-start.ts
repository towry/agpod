import { detectHiveMode, resolveCcHooksPath, saveSession, updateAgentStatus, validateHiveMode, withHiveLock } from "./hive-common.ts";

const { defineUnifiedHook } = await import(resolveCcHooksPath("lib/runtime/define-unified-hook.ts"));
const R = await import(resolveCcHooksPath("lib/runtime/results.ts"));

defineUnifiedHook({
  name: "hive-session-start",
  event: "SessionStart",
  supportedAgents: ["claude", "codex"],
  run() {
    const hive = detectHiveMode();
    if (!hive) return R.noop();

    return withHiveLock(hive, () => {
      const session = validateHiveMode(hive);
      if (!session) return R.noop();

      saveSession(hive, updateAgentStatus(session, hive, "idle"));
      return R.noop();
    });
  },
});
