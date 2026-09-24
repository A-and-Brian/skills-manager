import { useCallback, useEffect, useMemo, useState, type MouseEvent } from "react";
import { Link, useParams } from "react-router-dom";
import { Download, Loader2, RefreshCw, Server, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "../utils";
import { useApp } from "../context/AppContext";
import { AgentIcon } from "../components/AgentIcon";
import { SkillPickerRow } from "../components/SkillPickerRow";
import * as api from "../lib/tauri";
import type { ManagedSkill, RemoteHost, RemoteProbe, RemoteSkill, ToolInfo } from "../lib/tauri";
import { getErrorMessage } from "../lib/error";
import type { PickerStatus } from "../lib/skillPickerStatus";

/** Sources the remote can fetch on its own; anything else lives only here. */
const REMOTE_SOURCES = ["git", "skillssh"];

const rowButtonClass = "rounded-md px-2.5 py-1 text-[12px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50";
const rowButtonSecondary = `${rowButtonClass} bg-surface-hover text-secondary hover:bg-surface-active`;
const rowButtonPrimary = `${rowButtonClass} bg-accent-dark text-white hover:bg-accent`;

type Probe =
  | { state: "checking" }
  | { state: "ok"; result: RemoteProbe }
  | { state: "error"; message: string };

function sourceLabel(t: (key: string) => string, source: string) {
  return ["local", "import", "git", "skillssh"].includes(source)
    ? t(`mySkills.sourceFilter.${source}`)
    : source;
}

export function RemoteHostView() {
  const { hostId } = useParams<{ hostId: string }>();
  const { t } = useTranslation();
  const { remoteHosts, loading: appLoading } = useApp();
  const host = remoteHosts.find((h) => h.id === hostId);

  const [probe, setProbe] = useState<Probe>({ state: "checking" });
  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [skills, setSkills] = useState<RemoteSkill[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [installOpen, setInstallOpen] = useState(false);

  const agents = useMemo(() => tools.filter((tool) => tool.installed && tool.enabled), [tools]);
  const agent = agents.find((a) => a.key === selectedAgent) ?? agents[0];
  const canWrite = probe.state === "ok" && probe.result.compatible;

  const loadSkills = useCallback(async () => {
    if (!hostId) return;
    setSkills(await api.remoteHostSkills(hostId));
  }, [hostId]);

  const load = useCallback(async () => {
    if (!hostId) return;
    setLoading(true);
    setProbe({ state: "checking" });
    try {
      const result = await api.remoteHostProbe(hostId);
      setProbe({ state: "ok", result });
      const [remoteTools] = await Promise.all([api.remoteHostTools(hostId), loadSkills()]);
      setTools(remoteTools);
    } catch (e) {
      setProbe({ state: "error", message: getErrorMessage(e, t("common.error")) });
      setTools([]);
      setSkills([]);
    } finally {
      setLoading(false);
    }
  }, [hostId, loadSkills, t]);

  useEffect(() => {
    void load();
  }, [load]);

  // One remote write, then a fresh list so the badges reflect what the host says.
  const runWrite = async (key: string, action: () => Promise<unknown>, success: string) => {
    setBusy((prev) => new Set(prev).add(key));
    try {
      await action();
      toast.success(success);
      await loadSkills();
    } catch (e) {
      toast.error(getErrorMessage(e, t("common.error")));
    } finally {
      setBusy((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  const toggleDeploy = (skill: RemoteSkill, target: ToolInfo) => {
    if (!host) return;
    const deployed = skill.deployed_to.includes(target.key);
    const params = { name: skill.name, agent: target.display_name, host: host.name };
    void runWrite(
      skill.id,
      () =>
        deployed
          ? api.remoteHostUndeploy(host.id, skill.id, target.key)
          : api.remoteHostDeploy(host.id, skill.id, target.key),
      deployed ? t("remoteHosts.view.undeployed", params) : t("remoteHosts.view.deployed", params),
    );
  };

  const updateSkill = (skill: RemoteSkill) => {
    if (!host) return;
    void runWrite(
      skill.id,
      () => api.remoteHostUpdateSkill(host.id, skill.id),
      t("remoteHosts.view.updated", { name: skill.name, host: host.name }),
    );
  };

  if (!host) {
    if (appLoading) return null;
    return (
      <div className="app-page">
        <p className="text-[13px] text-muted">{t("remoteHosts.view.notFound")}</p>
        <Link to="/settings" className="mt-2 inline-block text-[13px] font-medium text-accent">
          {t("common.goToSettings")}
        </Link>
      </div>
    );
  }

  return (
    <div className="app-page">
      <div className="app-page-header flex flex-col gap-2.5 pb-3 pr-2">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="min-w-0 flex-1">
            <h1 className="app-page-title flex items-center gap-2.5">
              <Server className="h-5 w-5 text-accent" />
              {host.name}
              <span className="app-badge">{skills.length}</span>
            </h1>
            <p className="mt-1 flex flex-wrap items-center gap-x-1.5 text-[13px] text-muted">
              <span className="font-mono">{host.ssh_target}</span>
              <span>·</span>
              <ProbeState probe={probe} />
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <button
              onClick={() => void load()}
              disabled={loading}
              className="app-toolbar-button app-toolbar-button-secondary"
              title={t("settings.refresh")}
            >
              <RefreshCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
            </button>
            <button
              onClick={() => setInstallOpen(true)}
              disabled={!canWrite}
              className="app-toolbar-button app-toolbar-button-primary disabled:cursor-not-allowed disabled:opacity-50"
              title={canWrite ? undefined : t("remoteHosts.view.writesBlocked")}
            >
              <Download className="h-3.5 w-3.5" />
              {t("remoteHosts.view.install")}
            </button>
          </div>
        </div>

        {probe.state === "ok" && (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-[12px] text-muted">{t("remoteHosts.view.agents")}</span>
            {agents.length === 0 ? (
              <span className="text-[12px] text-faint">{t("remoteHosts.view.noAgents")}</span>
            ) : (
              agents.map((tool) => {
                const active = tool.key === agent?.key;
                return (
                  <button
                    key={tool.key}
                    onClick={() => setSelectedAgent(tool.key)}
                    className={cn(
                      "inline-flex items-center gap-1.5 rounded-full py-0.5 pl-0.5 pr-2.5 text-[12px] font-medium transition-colors",
                      active
                        ? "bg-accent text-white dark:bg-accent dark:text-white"
                        : "bg-surface-hover text-muted hover:text-secondary",
                    )}
                  >
                    <AgentIcon
                      agentKey={tool.key}
                      displayName={tool.display_name}
                      className="h-5 w-5 rounded-full border-0 bg-transparent"
                    />
                    {tool.display_name}
                  </button>
                );
              })
            )}
          </div>
        )}
      </div>

      {probe.state === "ok" && !probe.result.compatible && (
        <p className="mb-3 rounded-md bg-amber-500/10 px-3 py-2 text-[12px] text-amber-600 dark:text-amber-400">
          {t("remoteHosts.view.writesBlocked")}
        </p>
      )}

      {probe.state === "ok" && (
        <div className="app-panel divide-y divide-border-faint overflow-hidden">
          {skills.length === 0 ? (
            <p className="px-4 py-8 text-center text-[13px] text-muted">
              {loading ? t("common.loading") : t("remoteHosts.view.noSkills")}
            </p>
          ) : (
            skills.map((skill) => {
              const isBusy = busy.has(skill.id);
              const deployed = agent ? skill.deployed_to.includes(agent.key) : false;
              return (
                <div key={skill.id} className="flex items-center gap-3 px-4 py-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate text-[13px] font-medium text-primary">{skill.name}</span>
                      <span className="shrink-0 rounded-full bg-surface-hover px-1.5 py-0.5 text-[11px] font-medium text-muted">
                        {sourceLabel(t, skill.source_type)}
                      </span>
                    </div>
                    {skill.description && (
                      <p className="mt-0.5 truncate text-[12px] text-muted">{skill.description}</p>
                    )}
                    <div className="mt-1 flex flex-wrap items-center gap-1 text-[11px] text-muted">
                      {skill.deployed_to.length === 0 ? (
                        <span>{t("remoteHosts.view.notDeployed")}</span>
                      ) : (
                        <>
                          <span>{t("remoteHosts.view.deployedTo")}</span>
                          {skill.deployed_to.map((key) => (
                            <span
                              key={key}
                              className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-1.5 py-0.5 font-medium text-emerald-600 dark:text-emerald-400"
                            >
                              <AgentIcon agentKey={key} className="h-3.5 w-3.5 rounded-full border-0 bg-transparent" />
                              {tools.find((tool) => tool.key === key)?.display_name ?? key}
                            </span>
                          ))}
                        </>
                      )}
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    {isBusy && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted" />}
                    {REMOTE_SOURCES.includes(skill.source_type) && (
                      <button
                        onClick={() => updateSkill(skill)}
                        disabled={!canWrite || isBusy}
                        className={rowButtonSecondary}
                      >
                        {t("remoteHosts.view.update")}
                      </button>
                    )}
                    {agent && (
                      <button
                        onClick={() => toggleDeploy(skill, agent)}
                        disabled={!canWrite || isBusy}
                        className={deployed ? rowButtonSecondary : rowButtonPrimary}
                      >
                        {deployed
                          ? t("remoteHosts.view.undeploy", { agent: agent.display_name })
                          : t("remoteHosts.view.deploy", { agent: agent.display_name })}
                      </button>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>
      )}

      {installOpen && (
        <InstallSheet
          host={host}
          remoteSkills={skills}
          onClose={() => setInstallOpen(false)}
          onInstalled={loadSkills}
        />
      )}
    </div>
  );
}

function ProbeState({ probe }: { probe: Probe }) {
  const { t } = useTranslation();
  if (probe.state === "checking") {
    return (
      <span className="inline-flex items-center gap-1">
        <Loader2 className="h-3 w-3 animate-spin" />
        {t("common.loading")}
      </span>
    );
  }
  if (probe.state === "error") {
    return <span className="text-red-500 dark:text-red-400">{probe.message}</span>;
  }
  const { version, compatible, app_version } = probe.result;
  return compatible ? (
    <span className="text-emerald-600 dark:text-emerald-400">{t("remoteHosts.probeOk", { version })}</span>
  ) : (
    <span className="text-amber-600 dark:text-amber-400">
      {t("remoteHosts.probeIncompatible", { version, appVersion: app_version })}
    </span>
  );
}

/** Pick library skills for the host to install from their own source. */
function InstallSheet({
  host,
  remoteSkills,
  onClose,
  onInstalled,
}: {
  host: RemoteHost;
  remoteSkills: RemoteSkill[];
  onClose: () => void;
  onInstalled: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const { managedSkills } = useApp();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [installing, setInstalling] = useState(false);

  const allTags = useMemo(
    () => Array.from(new Set(managedSkills.flatMap((s) => s.tags))).sort(),
    [managedSkills],
  );
  const remoteRefs = useMemo(
    () => new Set(remoteSkills.map((s) => s.source_ref).filter(Boolean)),
    [remoteSkills],
  );

  const statusOf = (skill: ManagedSkill): PickerStatus => {
    if (!REMOTE_SOURCES.includes(skill.source_type) || !skill.source_ref) return "unavailable";
    return remoteRefs.has(skill.source_ref) ? "installed" : "available";
  };

  const toggle = (id: string) => (e: MouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const install = async () => {
    setInstalling(true);
    try {
      for (const skill of managedSkills.filter((s) => selected.has(s.id))) {
        try {
          await api.remoteHostInstall(host.id, skill.source_ref ?? "", skill.source_type);
          toast.success(t("remoteHosts.view.installed", { name: skill.name, host: host.name }));
        } catch (e) {
          toast.error(getErrorMessage(e, t("remoteHosts.view.installFailed", { name: skill.name })));
        }
      }
      await onInstalled();
      onClose();
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50">
      <div className="absolute inset-0 bg-black/40 backdrop-blur-[1px]" onClick={() => !installing && onClose()} />
      <div className="absolute right-0 top-0 flex h-full w-full max-w-[480px] flex-col overflow-hidden border-l border-border-subtle bg-bg-secondary shadow-2xl">
        <div className="flex shrink-0 items-start justify-between gap-3 border-b border-border-subtle px-5 py-4">
          <div className="min-w-0 flex-1">
            <h2 className="text-[14px] font-semibold text-primary">
              {t("remoteHosts.view.installTitle", { host: host.name })}
            </h2>
            <p className="mt-1 text-[12px] text-muted">{t("remoteHosts.view.installHint")}</p>
          </div>
          <button
            onClick={onClose}
            disabled={installing}
            className="shrink-0 rounded-md p-1.5 text-muted transition-colors hover:bg-surface-hover hover:text-secondary disabled:opacity-50"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto scrollbar-hide">
          {managedSkills.length === 0 ? (
            <div className="px-5 py-12 text-center text-[13px] text-muted">{t("addFromLibrary.emptyLibrary")}</div>
          ) : (
            <div className="divide-y divide-border-subtle">
              {managedSkills.map((skill) => (
                <SkillPickerRow
                  key={skill.id}
                  skill={skill}
                  status={statusOf(skill)}
                  allTags={allTags}
                  sourceLabel={sourceLabel(t, skill.source_type)}
                  selected={selected.has(skill.id)}
                  onToggle={toggle(skill.id)}
                  busy={installing && selected.has(skill.id)}
                  unavailableReason={t("remoteHosts.view.noRemoteSource")}
                />
              ))}
            </div>
          )}
        </div>

        <div className="shrink-0 border-t border-border-subtle bg-bg-secondary px-5 py-3">
          <button
            onClick={() => void install()}
            disabled={selected.size === 0 || installing}
            className="app-button-primary w-full"
          >
            {installing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
            {t("remoteHosts.view.installSelected", { count: selected.size })}
          </button>
        </div>
      </div>
    </div>
  );
}
