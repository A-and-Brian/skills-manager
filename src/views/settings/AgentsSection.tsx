import { useState, useCallback, useMemo } from "react";
import {
  FolderOpen,
  RefreshCw,
  Loader2,
  Pencil,
  RotateCcw,
  Plus,
  Trash2,
  X,
  Check,
  ChevronDown,
  ChevronRight,
  GripVertical,
} from "lucide-react";
import {
  DndContext,
  closestCenter,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  arrayMove,
  rectSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { confirm as dialogConfirm } from "@tauri-apps/plugin-dialog";
import { cn } from "../../utils";
import { useApp } from "../../context/AppContext";
import { AgentIcon } from "../../components/AgentIcon";
import { ToggleSwitch } from "../../components/ToggleSwitch";
import * as api from "../../lib/tauri";
import { getErrorMessage } from "../../lib/error";
import { ACTION_BUTTON_CLASS, FIELD_CLASS, compactHomePath, pickDirectory } from "./shared";

interface SortableAgentCardProps {
  agentKey: string;
  dragLabel: string;
  children: (dragHandle: React.ReactNode) => React.ReactNode;
}

function SortableAgentCard({ agentKey, dragLabel, children }: SortableAgentCardProps) {
  const {
    attributes,
    listeners,
    setNodeRef,
    setActivatorNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: agentKey });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : undefined,
  };

  const handle = (
    <button
      type="button"
      ref={setActivatorNodeRef}
      {...listeners}
      onClick={(e) => e.stopPropagation()}
      className="mt-0.5 flex shrink-0 cursor-grab items-center justify-center rounded text-faint outline-none transition-colors hover:text-muted active:cursor-grabbing"
      title={dragLabel}
      aria-label={dragLabel}
    >
      <GripVertical className="h-3.5 w-3.5" />
    </button>
  );

  return (
    <div ref={setNodeRef} style={style} {...attributes} className="h-full">
      {children(handle)}
    </div>
  );
}

interface AgentGroupDndProps {
  items: api.ToolInfo[];
  sensors: ReturnType<typeof useSensors>;
  dragLabel: string;
  onDragEnd: (event: DragEndEvent, groupKeys: string[]) => void;
  renderAgentCard: (agent: api.ToolInfo, dragHandle?: React.ReactNode) => React.ReactNode;
}

function AgentGroupDnd({ items, sensors, dragLabel, onDragEnd, renderAgentCard }: AgentGroupDndProps) {
  const groupKeys = items.map((t) => t.key);
  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragEnd={(e) => onDragEnd(e, groupKeys)}
    >
      <SortableContext items={groupKeys} strategy={rectSortingStrategy}>
        <div className="grid grid-cols-1 gap-1.5 md:grid-cols-2 xl:grid-cols-3">
          {items.map((agent) => (
            <SortableAgentCard key={agent.key} agentKey={agent.key} dragLabel={dragLabel}>
              {(handle) => renderAgentCard(agent, handle)}
            </SortableAgentCard>
          ))}
        </div>
      </SortableContext>
    </DndContext>
  );
}

export function AgentsSection() {
  const { t } = useTranslation();
  const { tools, refreshTools } = useApp();
  const [togglingTools, setTogglingTools] = useState<Set<string>>(new Set());
  const [refreshing, setRefreshing] = useState(false);
  // Agent path editing
  const [editingPathKey, setEditingPathKey] = useState<string | null>(null);
  const [editingPathValue, setEditingPathValue] = useState("");
  // Project path editing (custom agents only)
  const [editingProjectPathKey, setEditingProjectPathKey] = useState<string | null>(null);
  const [editingProjectPathValue, setEditingProjectPathValue] = useState("");
  // Custom agent dialog
  const [showAddCustom, setShowAddCustom] = useState(false);
  const [customName, setCustomName] = useState("");
  const [customPath, setCustomPath] = useState("");
  const [customProjectPath, setCustomProjectPath] = useState("");
  const [addingCustom, setAddingCustom] = useState(false);
  const [showMoreAgents, setShowMoreAgents] = useState(false);

  const startEditPath = useCallback((key: string, currentPath: string) => {
    setEditingPathKey(key);
    setEditingPathValue(currentPath);
  }, []);

  const handleSavePath = async () => {
    if (!editingPathKey || !editingPathValue.trim()) return;
    try {
      await api.setCustomToolPath(editingPathKey, editingPathValue.trim());
      await refreshTools();
      toast.success(t("settings.pathSaved"));
    } catch (e) {
      toast.error(String(e));
    } finally {
      setEditingPathKey(null);
    }
  };

  const startEditProjectPath = useCallback((key: string, currentPath: string | null) => {
    setEditingProjectPathKey(key);
    setEditingProjectPathValue(currentPath ?? "");
  }, []);

  const handleSaveProjectPath = async () => {
    if (!editingProjectPathKey) return;
    const trimmed = editingProjectPathValue.trim();
    try {
      await api.setCustomToolProjectPath(editingProjectPathKey, trimmed || null);
      await refreshTools();
      toast.success(t("settings.pathSaved"));
      setEditingProjectPathKey(null);
    } catch (e) {
      toast.error(String(e));
    }
  };

  const handleResetProjectPath = async (key: string) => {
    try {
      await api.resetCustomToolProjectPath(key);
      await refreshTools();
      toast.success(t("settings.projectPathReset"));
    } catch {
      toast.error(t("common.error"));
    }
  };

  const handleResetPath = async (key: string) => {
    try {
      await api.resetCustomToolPath(key);
      await refreshTools();
      toast.success(t("settings.pathReset"));
    } catch {
      toast.error(t("common.error"));
    }
  };

  const generateCustomAgentKey = useCallback(
    (name: string) => {
      const base = name
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "_")
        .replace(/^_+|_+$/g, "");
      const seed = base || "agent";
      const existingKeys = new Set(tools.map((tool) => tool.key));
      if (!existingKeys.has(seed)) return seed;
      let n = 2;
      while (existingKeys.has(`${seed}_${n}`)) n += 1;
      return `${seed}_${n}`;
    },
    [tools]
  );

  const handleAddCustomAgent = async () => {
    const trimName = customName.trim();
    const trimPath = customPath.trim();
    const trimProjectPath = customProjectPath.trim();
    if (!trimName || !trimPath) return;
    const trimKey = generateCustomAgentKey(trimName);
    setAddingCustom(true);
    try {
      await api.addCustomTool(trimKey, trimName, trimPath, trimProjectPath || undefined);
      await refreshTools();
      toast.success(t("settings.customAgentAdded"));
      setShowAddCustom(false);
      setCustomName("");
      setCustomPath("");
      setCustomProjectPath("");
    } catch (e) {
      toast.error(String(e));
    } finally {
      setAddingCustom(false);
    }
  };

  const handleRemoveCustomAgent = async (key: string, name: string) => {
    const shouldRemove = await dialogConfirm(t("settings.removeCustomAgentConfirm", { name }));
    if (!shouldRemove) return;
    try {
      await api.removeCustomTool(key);
      await refreshTools();
      toast.success(t("settings.customAgentRemoved"));
    } catch {
      toast.error(t("common.error"));
    }
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    await refreshTools();
    setRefreshing(false);
    toast.success(t("common.success"));
  };

  const handleToggleTool = async (key: string, enabled: boolean) => {
    setTogglingTools((prev) => new Set(prev).add(key));
    try {
      await api.setToolEnabled(key, enabled);
      await refreshTools();
    } catch {
      toast.error(t("common.error"));
    } finally {
      setTogglingTools((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  const handleToggleAllTools = async (enabled: boolean) => {
    try {
      await api.setAllToolsEnabled(enabled);
      await refreshTools();
      toast.success(t("common.success"));
    } catch {
      toast.error(t("common.error"));
    }
  };

  const installedTools = useMemo(() => tools.filter((tool) => tool.installed), [tools]);
  const enabledTools = useMemo(
    () => installedTools.filter((tool) => tool.enabled),
    [installedTools]
  );
  const customTools = useMemo(() => tools.filter((tool) => tool.is_custom), [tools]);
  const builtInTools = useMemo(() => tools.filter((tool) => !tool.is_custom), [tools]);
  // Grouped by what is actually on this machine rather than by a hand-kept
  // "mainstream" list. A settings page reader cares about the agents they have,
  // and that list stays correct without anyone re-curating it as products rise
  // and fall. Both groups keep the backend's order, which is ranked by how
  // widely used each agent is (see DEFAULT_PRIORITY_ORDER) and overridden by
  // whatever the user has dragged.
  const detectedTools = useMemo(
    () => builtInTools.filter((tool) => tool.installed),
    [builtInTools]
  );
  const undetectedTools = useMemo(
    () => builtInTools.filter((tool) => !tool.installed),
    [builtInTools]
  );

  const dragSensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }));

  const handleAgentDragEnd = useCallback(
    async (event: DragEndEvent, groupKeys: string[]) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;
      const oldIdx = groupKeys.indexOf(String(active.id));
      const newIdx = groupKeys.indexOf(String(over.id));
      if (oldIdx < 0 || newIdx < 0) return;

      const newGroupKeys = arrayMove(groupKeys, oldIdx, newIdx);
      const fullOrder = tools.map((t) => t.key);
      const groupKeySet = new Set(groupKeys);
      let cursor = 0;
      const newFullOrder = fullOrder.map((k) =>
        groupKeySet.has(k) ? newGroupKeys[cursor++] : k
      );

      try {
        await api.setToolOrder(newFullOrder);
        await refreshTools();
      } catch (e) {
        toast.error(getErrorMessage(e, t("common.error")));
      }
    },
    [tools, refreshTools, t]
  );

  const renderAgentCard = (agent: typeof tools[number], dragHandle?: React.ReactNode) => (
    <div
      className={cn(
        "group relative flex h-full flex-col gap-1.5 rounded-xl border px-3.5 py-3 transition-colors",
        agent.installed && agent.enabled
          ? "border-border bg-surface"
          : agent.installed
            ? "border-border-subtle bg-surface"
            : "border-border-subtle bg-bg-secondary"
      )}
    >
      <div className="flex items-start gap-2.5">
        {dragHandle}
        <AgentIcon
          agentKey={agent.key}
          displayName={agent.display_name}
          className="mt-px h-6 w-6 shrink-0 rounded-md"
        />

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3
              className={cn(
                "truncate text-[14px] font-semibold",
                agent.installed ? "text-primary" : "text-muted"
              )}
            >
              {agent.display_name}
            </h3>
            {/* Enabled/disabled is carried by the switch; only "not installed" adds info. */}
            {!agent.installed && (
              <span className="shrink-0 rounded-full bg-surface-hover px-2 py-0.5 text-[10px] font-medium text-muted">
                {t("settings.notInstalled")}
              </span>
            )}
          </div>

          <div className="mt-0.5 flex flex-wrap items-center gap-1">
            {agent.is_custom && (
              <span className="rounded-full bg-sky-500/10 px-2 py-0.5 text-[10px] font-medium text-sky-700 dark:text-sky-300">
                {t("settings.customAgent")}
              </span>
            )}
            {agent.is_custom && agent.project_relative_skills_dir && (
              <span className="rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                {t("settings.projectAgentSupported")}
              </span>
            )}
            {agent.has_path_override && !agent.is_custom && (
              <span className="rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-300">
                {t("settings.pathOverridden")}
              </span>
            )}
          </div>
        </div>

        {agent.is_custom && (
          <button
            onClick={() => handleRemoveCustomAgent(agent.key, agent.display_name)}
            className="mt-0.5 shrink-0 p-0.5 text-muted opacity-0 outline-none transition-opacity hover:text-red-500 group-hover:opacity-100"
            title={t("settings.removeCustomAgent")}
          >
            <Trash2 className="h-3 w-3" />
          </button>
        )}

        <ToggleSwitch
          className="mt-0.5"
          checked={agent.installed && agent.enabled}
          disabled={!agent.installed}
          loading={togglingTools.has(agent.key)}
          onChange={() => handleToggleTool(agent.key, !agent.enabled)}
          title={
            !agent.installed
              ? (t("settings.notInstalled") as string)
              : agent.enabled
                ? (t("settings.disableAgent") as string)
                : (t("settings.enableAgent") as string)
          }
        />
      </div>

      <div className="space-y-1">
        {/* Global skills path */}
        {editingPathKey === agent.key ? (
          <div className="flex items-center gap-1">
            <input
              type="text"
              value={editingPathValue}
              onChange={(e) => setEditingPathValue(e.target.value)}
              className="h-7 min-w-0 flex-1 rounded border border-border-subtle bg-background px-1.5 text-[12px] font-mono text-secondary outline-none focus:border-accent"
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter") handleSavePath();
                if (e.key === "Escape") setEditingPathKey(null);
              }}
            />
            <button
              onClick={() => pickDirectory(setEditingPathValue)}
              className="shrink-0 p-1 text-muted hover:text-accent outline-none"
              title={t("settings.selectFolder")}
            >
              <FolderOpen className="h-3 w-3" />
            </button>
            <button
              onClick={handleSavePath}
              className="shrink-0 p-1 text-emerald-500 hover:text-emerald-400 outline-none"
            >
              <Check className="h-3 w-3" />
            </button>
            <button
              onClick={() => setEditingPathKey(null)}
              className="shrink-0 p-1 text-muted hover:text-secondary outline-none"
            >
              <X className="h-3 w-3" />
            </button>
          </div>
        ) : (
          <div className="flex items-center gap-1">
            <p
              className="min-w-0 flex-1 truncate text-[12px] font-mono leading-tight text-muted"
              title={agent.skills_dir}
            >
              {compactHomePath(agent.skills_dir)}
            </p>
            <button
              type="button"
              onClick={() => startEditPath(agent.key, agent.skills_dir)}
              className="shrink-0 p-0.5 text-muted hover:text-accent outline-none opacity-0 transition-opacity group-hover:opacity-100"
              title={t("settings.editPath")}
            >
              <Pencil className="h-3 w-3" />
            </button>
            {agent.has_path_override && !agent.is_custom && (
              <button
                type="button"
                onClick={() => handleResetPath(agent.key)}
                className="shrink-0 p-0.5 text-muted hover:text-amber-500 outline-none opacity-0 transition-opacity group-hover:opacity-100"
                title={t("settings.resetPath")}
              >
                <RotateCcw className="h-3 w-3" />
              </button>
            )}
          </div>
        )}

        {/* Project-relative skills path — always rendered so every card is the
            same height, installed or not. */}
        {editingProjectPathKey === agent.key ? (
            <div className="flex items-center gap-1">
              <input
                type="text"
                value={editingProjectPathValue}
                onChange={(e) => setEditingProjectPathValue(e.target.value)}
                placeholder={t("settings.projectSkillsPathPlaceholder")}
                className="h-7 min-w-0 flex-1 rounded border border-border-subtle bg-background px-1.5 text-[12px] font-mono text-secondary outline-none focus:border-accent"
                autoFocus
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleSaveProjectPath();
                  if (e.key === "Escape") setEditingProjectPathKey(null);
                }}
              />
              <button
                onClick={handleSaveProjectPath}
                className="shrink-0 p-1 text-emerald-500 hover:text-emerald-400 outline-none"
              >
                <Check className="h-3 w-3" />
              </button>
              <button
                onClick={() => setEditingProjectPathKey(null)}
                className="shrink-0 p-1 text-muted hover:text-secondary outline-none"
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-1">
              <p
                className="min-w-0 flex-1 truncate text-[12px] font-mono leading-tight text-muted"
                title={agent.project_relative_skills_dir ?? t("settings.projectSkillsPathDesc")}
              >
                {agent.project_relative_skills_dir
                  ? !agent.is_custom && !agent.has_project_path_override
                    ? t("settings.projectSkillsPathDefault", {
                        path: agent.project_relative_skills_dir,
                      })
                    : t("settings.projectSkillsPathValue", {
                        path: agent.project_relative_skills_dir,
                      })
                  : t("settings.projectSkillsPathEmpty")}
              </p>
              <button
                type="button"
                onClick={() =>
                  startEditProjectPath(agent.key, agent.project_relative_skills_dir)
                }
                className="shrink-0 p-0.5 text-muted hover:text-accent outline-none opacity-0 transition-opacity group-hover:opacity-100"
                title={t("settings.editPath")}
              >
                <Pencil className="h-3 w-3" />
              </button>
              {!agent.is_custom && agent.has_project_path_override && (
                <button
                  type="button"
                  onClick={() => handleResetProjectPath(agent.key)}
                  className="shrink-0 p-0.5 text-muted hover:text-amber-500 outline-none opacity-0 transition-opacity group-hover:opacity-100"
                  title={t("settings.resetPath")}
                >
                  <RotateCcw className="h-3 w-3" />
                </button>
              )}
            </div>
          )}
      </div>
    </div>
  );

  return (
    <section>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="app-section-title">
            {t("settings.supportedAgents")} ({installedTools.length}/{tools.length})
          </h2>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <button
            onClick={() => setShowAddCustom(true)}
            className="flex items-center gap-1 text-[13px] text-accent hover:text-accent-light transition-colors font-medium outline-none"
          >
            <Plus className="w-3.5 h-3.5" />
            {t("settings.addCustomAgent")}
          </button>
          <button
            onClick={() => handleToggleAllTools(true)}
            className="text-[13px] text-accent hover:text-accent-light transition-colors font-medium outline-none"
          >
            {t("settings.enableAll")}
          </button>
          <button
            onClick={() => handleToggleAllTools(false)}
            className="text-[13px] text-muted hover:text-secondary transition-colors font-medium outline-none"
          >
            {t("settings.disableAll")}
          </button>
          <button
            onClick={handleRefresh}
            disabled={refreshing}
            className="flex items-center gap-1.5 text-[13px] text-accent hover:text-accent-light transition-colors font-medium outline-none"
          >
            {refreshing ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <RefreshCw className="w-3.5 h-3.5" />
            )}
            {t("settings.refresh")}
          </button>
        </div>
      </div>

      <div className="mb-3 flex flex-wrap items-center gap-3 text-[13px] text-muted">
        <span>{t("settings.detectedAgents")} <span className="font-medium text-secondary">{installedTools.length}</span></span>
        <span>{t("settings.enabledAgents")} <span className="font-medium text-secondary">{enabledTools.length}</span></span>
        <span>{t("settings.customAgents")} <span className="font-medium text-secondary">{customTools.length}</span></span>
      </div>

      {/* Add custom agent form */}
      {showAddCustom && (
        <div className="app-panel p-4 mb-3 space-y-2.5">
          <div className="flex items-center justify-between">
            <h3 className="text-[13px] font-medium text-secondary">{t("settings.addCustomAgent")}</h3>
            <button onClick={() => setShowAddCustom(false)} className="text-muted hover:text-secondary outline-none">
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
          <div>
            <label className="text-[12px] text-muted mb-1 block">{t("settings.agentName")}</label>
            <input
              type="text"
              value={customName}
              onChange={(e) => setCustomName(e.target.value)}
              placeholder={t("settings.agentNamePlaceholder")}
              className={`${FIELD_CLASS} w-full`}
            />
          </div>
          <div>
            <label className="text-[12px] text-muted mb-1 block">{t("settings.skillsPath")}</label>
            <div className="flex flex-wrap items-center gap-2">
              <input
                type="text"
                value={customPath}
                onChange={(e) => setCustomPath(e.target.value)}
                placeholder={t("settings.skillsPathPlaceholder")}
                className={`${FIELD_CLASS} min-w-0 flex-1 font-mono`}
              />
              <button
                onClick={() => pickDirectory(setCustomPath)}
                className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
              >
                <FolderOpen className="w-3 h-3" />
                {t("settings.selectFolder")}
              </button>
            </div>
          </div>
          <div>
            <label className="text-[12px] text-muted mb-1 block">
              {t("settings.projectSkillsPath")}
            </label>
            <input
              type="text"
              value={customProjectPath}
              onChange={(e) => setCustomProjectPath(e.target.value)}
              placeholder={t("settings.projectSkillsPathPlaceholder")}
              className={`${FIELD_CLASS} w-full font-mono`}
            />
            <p className="mt-1 text-[12px] text-muted">
              {t("settings.projectSkillsPathDesc")}
            </p>
          </div>
          <div className="flex justify-end">
            <button
              onClick={handleAddCustomAgent}
              disabled={addingCustom || !customName.trim() || !customPath.trim()}
              className={`${ACTION_BUTTON_CLASS} bg-accent text-white border-accent hover:opacity-90 disabled:opacity-50`}
            >
              {addingCustom ? <Loader2 className="w-3 h-3 animate-spin" /> : <Plus className="w-3 h-3" />}
              {t("settings.addAgent")}
            </button>
          </div>
        </div>
      )}

      <div className="space-y-4">
        {detectedTools.length > 0 && (
          <div>
            <div className="mb-2 flex items-center justify-between gap-2">
              <h3 className="text-[13px] font-medium text-secondary">{t("settings.detectedAgentsSection")}</h3>
              <span className="text-[12px] text-muted tabular-nums">{detectedTools.length}</span>
            </div>
            <AgentGroupDnd
              items={detectedTools}
              sensors={dragSensors}
              dragLabel={t("settings.dragToReorder")}
              onDragEnd={handleAgentDragEnd}
              renderAgentCard={renderAgentCard}
            />
          </div>
        )}

        {undetectedTools.length > 0 && (
          <div>
            <button
              type="button"
              onClick={() => setShowMoreAgents((value) => !value)}
              className="mb-2 inline-flex items-center gap-1.5 text-[13px] font-medium text-muted transition-colors hover:text-secondary outline-none"
            >
              {showMoreAgents ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
              {t("settings.otherAgentsSection", { count: undetectedTools.length })}
            </button>
            {showMoreAgents && (
              <AgentGroupDnd
                items={undetectedTools}
                sensors={dragSensors}
                dragLabel={t("settings.dragToReorder")}
                onDragEnd={handleAgentDragEnd}
                renderAgentCard={renderAgentCard}
              />
            )}
          </div>
        )}

        {customTools.length > 0 && (
          <div>
            <div className="mb-2 flex items-center justify-between gap-2">
              <h3 className="text-[13px] font-medium text-secondary">{t("settings.customAgentsSection")}</h3>
              <span className="text-[12px] text-muted">{customTools.length}</span>
            </div>
            <AgentGroupDnd
              items={customTools}
              sensors={dragSensors}
              dragLabel={t("settings.dragToReorder")}
              onDragEnd={handleAgentDragEnd}
              renderAgentCard={renderAgentCard}
            />
          </div>
        )}
      </div>
    </section>
  );
}
