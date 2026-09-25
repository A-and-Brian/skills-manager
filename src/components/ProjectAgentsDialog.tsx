import { useEffect, useState } from "react";
import { AlertTriangle, Link2, Loader2, Minus, Plus, SlidersHorizontal, Square, SquareCheck, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "../utils";
import { AgentIcon } from "./AgentIcon";
import { DeployModePicker } from "./DeployModePicker";
import * as api from "../lib/tauri";
import type { AgentChangePlan, ProjectAgentTarget, ProjectDeployMode, SkillConversion } from "../lib/tauri";
import { getErrorMessage } from "../lib/error";
import { isVendoredSkillsDir } from "../lib/projectSkillGroups";

interface Props {
  open: boolean;
  projectId: string;
  deployMode: ProjectDeployMode;
  targets: ProjectAgentTarget[];
  onClose: () => void;
  onApplied: () => Promise<void>;
}

type ModeChange = { to: "copy"; conversions: SkillConversion[] } | { to: "link" };

/**
 * A project's settings: how skills are deployed, and which agents get them.
 * Both reach the skills already in the project, so each change is previewed
 * or confirmed before it runs; skills whose agents were picked by hand keep them.
 */
export function ProjectAgentsDialog({ open, projectId, deployMode, targets, onClose, onApplied }: Props) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [plan, setPlan] = useState<AgentChangePlan | null>(null);
  const [modeChange, setModeChange] = useState<ModeChange | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setSelected(new Set(targets.filter((target) => target.selected).map((target) => target.key)));
    setPlan(null);
    setModeChange(null);
  }, [open, targets]);

  useEffect(() => {
    if (!open || busy) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, busy, onClose]);

  if (!open) return null;

  // Keep the list in the order agents are shown elsewhere.
  const agentKeys = targets.filter((target) => selected.has(target.key)).map((target) => target.key);
  const nameOf = (key: string) => targets.find((target) => target.key === key)?.display_name ?? key;
  const hasChanges = plan?.skills.some((skill) => skill.adds.length > 0 || skill.removes.length > 0) ?? false;

  const toggle = (key: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    try {
      await action();
    } catch (e) {
      toast.error(getErrorMessage(e, t("common.error")));
    } finally {
      setBusy(false);
    }
  };

  const handleReview = () => run(async () => {
    setPlan(await api.previewProjectAgentChange(projectId, agentKeys));
  });

  const handlePickMode = (mode: ProjectDeployMode) => {
    if (mode === deployMode) return;
    if (mode === "link") {
      setModeChange({ to: "link" });
      return;
    }
    void run(async () => {
      setModeChange({ to: "copy", conversions: await api.previewProjectConvertToCopy(projectId) });
    });
  };

  const handleApply = () => run(async () => {
    if (hasChanges) {
      const outcomes = await api.applyProjectAgentChange(projectId, agentKeys);
      const added = outcomes.reduce((sum, outcome) => sum + outcome.added.length, 0);
      const removed = outcomes.reduce((sum, outcome) => sum + outcome.removed.length, 0);
      const failures = outcomes.flatMap((outcome) => outcome.failed);
      toast.success(t("project.agentsDialog.applied", { added, removed }));
      if (failures.length > 0) {
        toast.error(`${t("project.agentsDialog.failed", { count: failures.length })} — ${failures[0].error}`);
      }
    } else {
      await api.setProjectAgentKeys(projectId, agentKeys);
      toast.success(t("project.agentsDialog.saved"));
    }
    await onApplied();
    onClose();
  });

  const handleModeChange = () => run(async () => {
    if (modeChange?.to === "link") {
      await api.setProjectDeployMode(projectId, "link");
      toast.success(t("project.settings.switchedToLink"));
    } else {
      const outcomes = await api.applyProjectConvertToCopy(projectId);
      toast.success(t("project.settings.converted", {
        vendored: outcomes.filter((outcome) => outcome.vendored).length,
        relinked: outcomes.reduce((sum, outcome) => sum + outcome.relinked.length, 0),
        kept: outcomes.reduce((sum, outcome) => sum + outcome.kept.length, 0),
      }));
      const errors = outcomes.flatMap((outcome) => [
        ...(outcome.error ? [`${outcome.name}: ${outcome.error}`] : []),
        ...outcome.failed.map((failure) => `${outcome.name} (${nameOf(failure.agent)}): ${failure.error}`),
      ]);
      if (errors.length > 0) {
        toast.error(`${t("project.settings.convertFailed", { count: errors.length })} — ${errors[0]}`);
      }
    }
    await onApplied();
    onClose();
  });

  const title = modeChange
    ? t(modeChange.to === "copy" ? "project.settings.convertTitle" : "project.settings.linkTitle")
    : plan
      ? t("project.agentsDialog.reviewTitle")
      : t("project.agentsDialog.title");
  const primaryLabel = modeChange
    ? t(modeChange.to === "copy" ? "project.settings.convert" : "project.settings.switchToLink")
    : !plan
      ? t("project.agentsDialog.review")
      : hasChanges
        ? t("project.agentsDialog.apply")
        : t("common.save");
  const back = modeChange ? () => setModeChange(null) : plan ? () => setPlan(null) : onClose;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/70 backdrop-blur-sm" onClick={busy ? undefined : onClose} />
      <div className="relative flex max-h-[80vh] w-full max-w-[520px] flex-col rounded-xl border border-border bg-surface p-5 shadow-2xl">
        <div className="mb-3 flex items-center justify-between">
          <h2 className="flex items-center gap-2 text-[13px] font-semibold text-primary">
            <SlidersHorizontal className="h-4 w-4 text-accent-light" />
            {title}
          </h2>
          <button
            onClick={onClose}
            disabled={busy}
            className="rounded p-1 text-muted outline-none transition-colors hover:text-secondary"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {modeChange?.to === "copy" ? (
          <ConvertPreview conversions={modeChange.conversions} nameOf={nameOf} />
        ) : modeChange?.to === "link" ? (
          <p className="flex gap-2 text-[12px] text-muted">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-400" />
            {t("project.settings.linkWarning")}
          </p>
        ) : plan ? (
          <div className="min-h-0 flex-1 overflow-y-auto text-[12px]">
            {!hasChanges && (
              <p className="mb-2 text-muted">{t("project.agentsDialog.noChanges")}</p>
            )}
            {plan.overridden > 0 && (
              <p className="mb-2 text-muted">{t("project.agentsDialog.overridden", { count: plan.overridden })}</p>
            )}
            <div className="flex flex-col gap-1.5">
              {plan.skills.map((skill) => (
                <div key={skill.relative_path} className="rounded-md border border-border-subtle bg-bg-secondary px-2.5 py-2">
                  <div className="mb-1 truncate font-medium text-secondary" title={skill.relative_path}>
                    {skill.name}
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {skill.adds.map((agent) => (
                      <span key={`add-${agent}`} className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-emerald-700 dark:text-emerald-300">
                        <Plus className="h-3 w-3" />
                        {nameOf(agent)}
                      </span>
                    ))}
                    {skill.removes.map((agent) => (
                      <span key={`remove-${agent}`} className="inline-flex items-center gap-1 rounded-full bg-red-500/10 px-2 py-0.5 text-red-600 dark:text-red-300">
                        <Minus className="h-3 w-3" />
                        {nameOf(agent)}
                      </span>
                    ))}
                  </div>
                  {skill.skipped.map((item) => (
                    <div key={`skip-${item.agent}`} className="mt-1 text-muted">
                      {t("project.agentsDialog.skippedAgent", {
                        agent: nameOf(item.agent),
                        reason: t(`project.agentsDialog.skipReason.${item.reason}`),
                      })}
                    </div>
                  ))}
                </div>
              ))}
            </div>
          </div>
        ) : (
          <>
            <div className="mb-1.5 text-[12px] font-medium text-secondary">{t("project.settings.modeLabel")}</div>
            <DeployModePicker value={deployMode} onChange={handlePickMode} disabled={busy} className="mb-4" />
            <div className="mb-1 text-[12px] font-medium text-secondary">{t("project.settings.agentsLabel")}</div>
            <p className="mb-1 text-[12px] text-muted">{t("project.agentsDialog.description")}</p>
            <p className="mb-3 text-[12px] text-muted">{t("project.agentsDialog.sharedFolderHint")}</p>
            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="grid gap-1.5">
                {targets.map((target) => {
                  const checked = selected.has(target.key);
                  const available = target.installed && target.enabled;
                  const badge = !target.installed
                    ? t("mySkills.agentToggleNotInstalled")
                    : !target.enabled
                      ? t("mySkills.agentToggleDisabledGlobally")
                      : deployMode === "copy" && isVendoredSkillsDir(target.relative_skills_dir)
                        ? t("project.vendored.badge")
                        : null;
                  return (
                    <button
                      key={target.key}
                      type="button"
                      onClick={() => toggle(target.key)}
                      // An unavailable agent can still be deselected, never newly chosen.
                      disabled={!available && !checked}
                      className={cn(
                        "flex w-full items-center gap-2 rounded-md border px-2 py-1.5 text-left text-[12px] transition-colors",
                        checked ? "border-border bg-surface" : "border-border-subtle bg-bg-secondary",
                        !available && !checked ? "opacity-55" : "hover:bg-surface-hover"
                      )}
                    >
                      <span className="shrink-0">
                        {checked
                          ? <SquareCheck className="h-3.5 w-3.5 text-accent" />
                          : <Square className="h-3.5 w-3.5 text-faint" />}
                      </span>
                      <AgentIcon agentKey={target.key} displayName={target.display_name} className="h-5 w-5 rounded-[4px]" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-medium text-secondary">{target.display_name}</span>
                        <span className="block truncate font-mono text-[11px] text-faint">{target.relative_skills_dir}</span>
                      </span>
                      {badge && <span className="shrink-0 text-[11px] text-muted">{badge}</span>}
                    </button>
                  );
                })}
              </div>
            </div>
          </>
        )}

        <div className="flex justify-end gap-2 pt-5">
          <button
            onClick={back}
            disabled={busy}
            className="rounded-lg px-3 py-1.5 text-[13px] font-medium text-tertiary outline-none transition-colors hover:bg-surface-hover hover:text-secondary"
          >
            {modeChange || plan ? t("project.agentsDialog.back") : t("common.cancel")}
          </button>
          <button
            onClick={modeChange ? handleModeChange : plan ? handleApply : handleReview}
            disabled={busy}
            className="inline-flex items-center gap-1.5 rounded-lg border border-accent-border bg-accent-dark px-3 py-1.5 text-[13px] font-medium text-white outline-none transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          >
            {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
            {primaryLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

/** What converting to a vendored copy does, per skill and per agent copy. */
function ConvertPreview({
  conversions,
  nameOf,
}: {
  conversions: SkillConversion[];
  nameOf: (key: string) => string;
}) {
  const { t } = useTranslation();
  const variants = conversions.flatMap((conversion) => conversion.variants);
  const relinked = variants.filter((variant) => variant.action === "relink").length;

  return (
    <div className="min-h-0 flex-1 overflow-y-auto text-[12px]">
      <p className="mb-2 text-muted">{t("project.settings.convertIntro")}</p>
      <p className="mb-2 font-medium text-secondary">
        {conversions.length === 0
          ? t("project.settings.convertEmpty")
          : t("project.settings.convertSummary", {
            vendored: conversions.filter((conversion) => conversion.copy_from).length,
            relinked,
            kept: variants.length - relinked,
          })}
      </p>
      <div className="flex flex-col gap-1.5">
        {conversions.map((conversion) => (
          <div key={conversion.relative_path} className="rounded-md border border-border-subtle bg-bg-secondary px-2.5 py-2">
            <div className="truncate font-medium text-secondary" title={conversion.relative_path}>
              {conversion.name}
            </div>
            {conversion.copy_from && (
              <div className="truncate font-mono text-[11px] text-faint" title={conversion.copy_from}>
                {t("project.settings.copyFrom", { path: conversion.copy_from })}
              </div>
            )}
            <div className="mt-1 flex flex-wrap gap-1.5">
              {conversion.variants.filter((variant) => variant.action === "relink").map((variant) => (
                <span key={variant.agent} className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-emerald-700 dark:text-emerald-300">
                  <Link2 className="h-3 w-3" />
                  {nameOf(variant.agent)}
                </span>
              ))}
            </div>
            {conversion.variants.filter((variant) => variant.action !== "relink").map((variant) => (
              <div key={variant.agent} className="mt-1 text-muted">
                {t("project.agentsDialog.skippedAgent", {
                  agent: nameOf(variant.agent),
                  reason: t(`project.settings.keepReason.${variant.action}`),
                })}
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
