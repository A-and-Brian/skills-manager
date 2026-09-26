import { useCallback, useEffect, useState } from "react";
import * as api from "../lib/tauri";
import { parseLastUsedAgents } from "../lib/projectSkillGroups";

const projectLastUsedAgentsKey = (projectId: string) =>
  `project_last_used_export_agents:${projectId}`;

/** The agents last used to add skills to project `id`, remembered per project. */
export function useLastUsedExportAgents(id: string | undefined) {
  const [lastUsedExportAgents, setLastUsedExportAgents] = useState<string[] | null>(null);
  useEffect(() => {
    if (!id) return;
    let cancelled = false;
    api.getSettings(projectLastUsedAgentsKey(id))
      .then((raw) => {
        if (!cancelled) setLastUsedExportAgents(parseLastUsedAgents(raw));
      })
      .catch(() => {
        if (!cancelled) setLastUsedExportAgents(null);
      });
    return () => {
      cancelled = true;
    };
  }, [id]);

  const handlePersistLastUsedAgents = useCallback(
    (agents: string[]) => {
      setLastUsedExportAgents(agents);
      if (id) {
        void api.setSettings(projectLastUsedAgentsKey(id), JSON.stringify(agents)).catch(() => {});
      }
    },
    [id],
  );

  return { lastUsedExportAgents, handlePersistLastUsedAgents };
}
