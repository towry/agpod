import {
  detectHiveMode,
  displayMessageToPane,
  queenPaneExists,
  resolveCcHooksPath,
  saveSession,
  updateAgentStatus,
  validateHiveMode,
  withHiveLock,
} from "./hive-common.ts";

const { parseAgent } = await import(resolveCcHooksPath("lib/runtime/agent.ts"));
const { getLastAssistantMessage } = await import(resolveCcHooksPath("lib/transcript/last-message.ts"));
const { defineUnifiedHook } = await import(resolveCcHooksPath("lib/runtime/define-unified-hook.ts"));
const R = await import(resolveCcHooksPath("lib/runtime/results.ts"));

defineUnifiedHook({
  name: "hive-agent-stop",
  event: "Stop",
  supportedAgents: ["claude", "codex"],
  async run(ctx) {
    const hive = detectHiveMode();
    if (!hive) return R.noop();

    return withHiveLock(hive, async () => {
      const session = validateHiveMode(hive);
      if (!session) return R.noop();

      saveSession(hive, updateAgentStatus(session, hive, "idle"));

      if (!queenPaneExists(hive.queenPaneId)) {
        return R.noop();
      }

      const agent = parseAgent();
      const summary = await getLastAssistantMessage({
        agent,
        transcriptPath: ctx.transcriptPath,
      });
      const message = `HIVE_DONE:${hive.workerPaneId}:${hive.workerName ?? ""}:${summary}`;
      displayMessageToPane(hive.queenPaneId, message);
      return R.noop();
    });
  },
});
