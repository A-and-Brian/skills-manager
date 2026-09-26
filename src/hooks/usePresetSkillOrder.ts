import { useCallback, useEffect, useState } from "react";
import * as api from "../lib/tauri";
import type { ManagedSkill, Preset } from "../lib/tauri";

/**
 * The saved skill order of the viewed preset. `reorder` applies the new order
 * optimistically and reloads the saved one if the save fails.
 */
export function usePresetSkillOrder(viewedPreset: Preset | null, skills: ManagedSkill[]) {
  const [presetSkillOrder, setPresetSkillOrder] = useState<string[]>([]);

  // Fetch sort order whenever active preset changes
  useEffect(() => {
    if (!viewedPreset) {
      // Moved verbatim from MySkills, where this lint did not reach it. A
      // render-time reset would also discard a late fetch result.
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setPresetSkillOrder([]);
      return;
    }
    api.getPresetSkillOrder(viewedPreset.id).then(setPresetSkillOrder).catch(() => {});
  }, [viewedPreset, skills]);

  const reorder = useCallback(async (presetId: string, skillIds: string[]) => {
    // Optimistic update
    setPresetSkillOrder(skillIds);

    try {
      await api.reorderPresetSkills(presetId, skillIds);
    } catch {
      // Revert on failure
      await api.getPresetSkillOrder(presetId).then(setPresetSkillOrder).catch(() => {});
    }
  }, []);

  return { presetSkillOrder, reorder };
}
