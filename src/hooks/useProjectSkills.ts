import { useCallback, useEffect, useRef, useState } from "react";
import * as api from "../lib/tauri";
import type { ProjectSkill } from "../lib/tauri";

/** A project's scanned skills, loaded when `id` changes and on `loadSkills`. */
export function useProjectSkills(id: string | undefined) {
  const [skills, setSkills] = useState<ProjectSkill[]>([]);
  const [loading, setLoading] = useState(true);

  // Scanning a project is slow enough that switching projects can let the older
  // scan land last, swapping another project's skills in under this route — and
  // now also pruning this project's tag filter against the other one's tags.
  // Same request-id guard as WorkspaceView's local-skill load.
  const skillsRequestRef = useRef(0);
  const loadSkills = useCallback(async () => {
    if (!id) return;
    const requestId = ++skillsRequestRef.current;
    setLoading(true);
    try {
      const result = await api.getProjectSkills(id);
      if (skillsRequestRef.current === requestId) setSkills(result);
    } catch (e) {
      console.error("Failed to load project skills:", e);
    } finally {
      if (skillsRequestRef.current === requestId) setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

  return { skills, loading, loadSkills };
}
