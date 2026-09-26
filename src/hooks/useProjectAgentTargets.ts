import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import type { ProjectAgentTarget } from "../lib/tauri";

/** Project `id`'s agent targets, reloaded when its agent selection changes. */
export function useProjectAgentTargets(id: string | undefined, agentKeys: string[] | null) {
  const [projectAgentTargets, setProjectAgentTargets] = useState<ProjectAgentTarget[]>([]);
  // Targets carry the selection, so they reload when it changes.
  const agentKeysSignal = JSON.stringify(agentKeys);

  useEffect(() => {
    let cancelled = false;
    const loadProjectAgentTargets = async () => {
      if (!id) return;
      try {
        const result = await api.getProjectAgentTargets(id);
        if (!cancelled) {
          setProjectAgentTargets(result);
        }
      } catch (e) {
        console.error("Failed to load project agent targets:", e);
      }
    };
    loadProjectAgentTargets();
    return () => {
      cancelled = true;
    };
  }, [id, agentKeysSignal]);

  return projectAgentTargets;
}
