import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { UploadCloud, Github, Box } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "../utils";
import { useApp } from "../context/AppContext";
import * as api from "../lib/tauri";
import type { SkillsShSkill, BatchImportResult } from "../lib/tauri";
import { useNavigate, useSearch } from "@tanstack/react-router";
import type { InstallTab } from "./installSearch";
import { listenOnActiveHost } from "../lib/hostEvents";
import { pickPath } from "../lib/pickPath";
import { useMarketSearch } from "../hooks/useMarketSearch";
import { useSourceOverflow } from "../hooks/useSourceOverflow";
import { useGitPreview } from "../hooks/useGitPreview";
import { useLocalScan } from "../hooks/useLocalScan";
import { findInstalledByGitUrl as findInstalledSkillByGitUrl } from "../lib/gitUrl";
import { MarketTab } from "../components/MarketTab";
import { GitInstallTab } from "../components/GitInstallTab";
import { LocalInstallTab } from "../components/LocalInstallTab";
import { GitPreviewDialog } from "../components/GitPreviewDialog";
import { getErrorMessage, getErrorKind } from "../lib/error";

export function InstallSkills() {
  const { t } = useTranslation();
  const { refreshPresets, refreshManagedSkills, managedSkills, openSkillDetailById } = useApp();
  const navigate = useNavigate();
  const { tab: tabParam } = useSearch({ from: "/install" });
  const [activeTab, setActiveTab] = useState<InstallTab>("market");
  const market = useMarketSearch(activeTab === "market");
  const [installing, setInstalling] = useState<string | null>(null);
  const {
    gitUrl,
    setGitUrl,
    gitLoading,
    gitCancelKey,
    gitPreview,
    gitSelections,
    setGitSelections,
    gitConfirmLoading,
    handleGitPreview,
    handleGitPreviewClose,
    handleGitConfirm,
  } = useGitPreview();
  const {
    scanResult,
    scanLoading,
    localError,
    setLocalError,
    runScan,
    runScanSilent,
  } = useLocalScan(activeTab === "local");
  const [importingPaths, setImportingPaths] = useState<Set<string>>(new Set());
  const [importingAll, setImportingAll] = useState(false);
  const [renameEditing, setRenameEditing] = useState<Record<string, string>>({});
  const sourceOverflow = useSourceOverflow(market.sourceOptions);

  const managedSkillsRef = useRef(managedSkills);
  managedSkillsRef.current = managedSkills;

  const goToSkill = useCallback((skillName: string) => {
    // Use ref to get the latest managedSkills after refresh
    const skills = managedSkillsRef.current;
    const skill = skills.find(
      (s) => s.name === skillName || s.source_ref === skillName
    );
    if (skill) {
      openSkillDetailById(skill.id);
    }
    navigate({ to: "/my-skills" });
  }, [navigate, openSkillDetailById]);

  const installedSourceRefs = useMemo(() => {
    const set = new Set<string>();
    for (const skill of managedSkills) {
      if (skill.source_type === "skillssh" && skill.source_ref) {
        set.add(skill.source_ref);
      }
    }
    return set;
  }, [managedSkills]);

  const findInstalledByGitUrl = useCallback(
    (url: string) => findInstalledSkillByGitUrl(managedSkills, url),
    [managedSkills]
  );

  useEffect(() => {
    if (tabParam) {
      setActiveTab(tabParam);
    }
  }, [tabParam]);

  const switchTab = (tab: InstallTab) => {
    setActiveTab(tab);
    navigate({ to: "/install", search: { tab } });
  };

  const warnRejected = (results: PromiseSettledResult<unknown>[], label: string) => {
    for (const r of results) {
      if (r.status === "rejected") console.warn(`${label} failed:`, r.reason);
    }
  };

  const installLocalSource = async (sourcePath: string) => {
    const name = sourcePath.split("/").pop() || sourcePath;
    const toastId = toast.loading(t("install.toast.installing", { name }));
    try {
      await api.installLocal(sourcePath);
    } catch (e) {
      const message = getErrorMessage(e, t("common.error"));
      setLocalError(message);
      toast.error(message, { id: toastId });
      return;
    }
    // Install succeeded — post-install refresh is best-effort and must not
    // surface as an install failure.
    const results = await Promise.allSettled([
      refreshPresets(),
      refreshManagedSkills(),
      runScanSilent(),
    ]);
    warnRejected(results, "post-install refresh");
    toast.success(t("install.toast.success", { name }), {
      id: toastId,
      action: {
        label: t("install.toast.view"),
        onClick: () => goToSkill(name),
      },
    });
  };

  const handleLocalFolderInstall = async () => {
    try {
      const selected = await pickPath({ directory: true });
      if (!selected) return;
      installLocalSource(selected);
    } catch (error: unknown) {
      const message = getErrorMessage(error, t("common.error"));
      setLocalError(message);
      toast.error(message);
    }
  };

  const handleLocalFileInstall = async () => {
    try {
      const selected = await pickPath({ files: ["zip", "skill"], filterName: "Skills" });
      if (!selected) return;
      installLocalSource(selected);
    } catch (error: unknown) {
      const message = getErrorMessage(error, t("common.error"));
      setLocalError(message);
      toast.error(message);
    }
  };

  const handleBatchImportFolder = async () => {
    let unlisten: (() => void) | null = null;
    try {
      const selected = await pickPath({ directory: true });
      if (!selected) return;

      const toastId = toast.loading(t("install.local.batchImporting"));

      unlisten = await listenOnActiveHost<{ current: number; total: number; name: string }>(
        "batch-import-progress",
        (event) => {
          const { current, total, name } = event.payload;
          toast.loading(
            t("install.local.batchProgress", { current, total, name }),
            { id: toastId }
          );
        }
      );

      const result: BatchImportResult = await api.batchImportFolder(selected);

      if (result.errors.length > 0) {
        const previewErrors = result.errors.slice(0, 3).join("; ");
        const remaining = result.errors.length - 3;
        const detail = remaining > 0 ? `${previewErrors}; +${remaining} more` : previewErrors;
        toast.error(
          `${t("install.local.batchErrors", { count: result.errors.length })}: ${detail}`,
          { id: toastId }
        );
      } else if (result.imported === 0) {
        toast.info(
          t("install.local.batchAllSkipped", { skipped: result.skipped }),
          { id: toastId }
        );
      } else {
        toast.success(
          t("install.local.batchSuccess", {
            imported: result.imported,
            skipped: result.skipped,
          }),
          { id: toastId }
        );
      }

      await Promise.all([refreshPresets(), refreshManagedSkills()]);
      runScan();
    } catch (error: unknown) {
      const message = getErrorMessage(error, t("common.error"));
      setLocalError(message);
      toast.error(message);
    } finally {
      unlisten?.();
    }
  };

  const handleInstallSkillssh = async (skill: SkillsShSkill) => {
    const displayName = skill.name || skill.skill_id;
    const cancelKey = `${skill.source}/${skill.skill_id}`;
    setInstalling(skill.id);

    const toastId = toast.loading(t("install.toast.cloning"));
    let unlisten: (() => void) | null = null;

    try {
      unlisten = await listenOnActiveHost<{ skill_id: string; phase: string; detail?: string }>(
        "install-progress",
        (event) => {
          if (event.payload.skill_id !== cancelKey) return;
          if (event.payload.phase === "cloning") {
            const detail = event.payload.detail?.trim();
            const msg = detail
              ? `${t("install.toast.cloning")}\n${detail}`
              : t("install.toast.cloning");
            toast.loading(msg, { id: toastId });
          } else if (event.payload.phase === "installing") {
            toast.loading(t("install.toast.installing", { name: displayName }), { id: toastId });
          }
        }
      );
      await api.installFromSkillssh(skill.source, skill.skill_id);
      await Promise.all([refreshPresets(), refreshManagedSkills()]);
      toast.success(t("install.toast.success", { name: displayName }), {
        id: toastId,
        action: {
          label: t("install.toast.view"),
          onClick: () => goToSkill(displayName),
        },
      });
    } catch (error: unknown) {
      if (getErrorKind(error) === "cancelled") {
        toast.info(t("install.toast.cancelled"), { id: toastId });
      } else {
        toast.error(getErrorMessage(error, t("common.error")), { id: toastId });
      }
    } finally {
      setInstalling(null);
      unlisten?.();
    }
  };

  const handleCancelInstall = (cancelKey: string) => {
    api.cancelInstall(cancelKey).catch(() => {
      // Ignore race: install may have completed before cancel request arrives.
    });
  };

  const handleImportDiscovered = async (sourcePath: string, name: string) => {
    setImportingPaths((prev) => new Set(prev).add(sourcePath));
    try {
      try {
        await api.importExistingSkill(sourcePath, name);
      } catch (error: unknown) {
        toast.error(getErrorMessage(error, t("common.error")));
        return;
      }
      toast.success(t("install.scan.importedOne", { name }));
      const results = await Promise.allSettled([
        refreshPresets(),
        refreshManagedSkills(),
        runScanSilent(),
      ]);
      warnRejected(results, "post-import refresh");
    } finally {
      setImportingPaths((prev) => {
        const next = new Set(prev);
        next.delete(sourcePath);
        return next;
      });
    }
  };

  const handleImportAllDiscovered = async () => {
    setImportingAll(true);
    try {
      try {
        await api.importAllDiscovered();
      } catch (error: unknown) {
        toast.error(getErrorMessage(error, t("common.error")));
        return;
      }
      toast.success(t("install.scan.importedAll"));
      const results = await Promise.allSettled([
        refreshPresets(),
        refreshManagedSkills(),
        runScanSilent(),
      ]);
      warnRejected(results, "post-import refresh");
    } finally {
      setImportingAll(false);
    }
  };

  return (
    <div className="app-page gap-4">
      <div className="app-page-header border-b-0 pb-0">
        <h1 className="app-page-title mb-4">{t("install.title")}</h1>
        <div className="flex gap-1 border-b border-border-subtle">
          {[
            { id: "market" as const, label: t("install.browseMarket"), icon: Box },
            { id: "local" as const, label: t("install.localInstall"), icon: UploadCloud },
            { id: "git" as const, label: t("install.gitInstall"), icon: Github },
          ].map((tab) => {
            const Icon = tab.icon;
            const isActive = activeTab === tab.id;
            return (
              <button
                key={tab.id}
                onClick={() => switchTab(tab.id)}
                className={cn(
                  "mr-4 flex items-center gap-1.5 border-b-2 px-1 pb-1.5 text-[13px] font-medium transition-colors outline-none",
                  isActive
                    ? "border-accent text-accent"
                    : "border-transparent text-muted hover:text-tertiary"
                )}
              >
                <Icon className="h-3.5 w-3.5" />
                {tab.label}
              </button>
            );
          })}
        </div>
      </div>

      {activeTab === "market" && (
        <MarketTab
          market={market}
          sourceOverflow={sourceOverflow}
          installedSourceRefs={installedSourceRefs}
          installing={installing}
          onInstall={handleInstallSkillssh}
          onCancelInstall={handleCancelInstall}
        />
      )}

      {activeTab === "local" && (
        <LocalInstallTab
          scanResult={scanResult}
          scanLoading={scanLoading}
          localError={localError}
          importingPaths={importingPaths}
          importingAll={importingAll}
          renameEditing={renameEditing}
          setRenameEditing={setRenameEditing}
          onInstallFolder={handleLocalFolderInstall}
          onInstallArchive={handleLocalFileInstall}
          onBatchImport={handleBatchImportFolder}
          onRescan={runScan}
          onImportAll={handleImportAllDiscovered}
          onImportOne={handleImportDiscovered}
        />
      )}

      {activeTab === "git" && (
        <GitInstallTab
          gitUrl={gitUrl}
          gitLoading={gitLoading}
          gitCancelKey={gitCancelKey}
          findInstalledByGitUrl={findInstalledByGitUrl}
          onGitUrlChange={setGitUrl}
          onPreview={handleGitPreview}
          onCancelInstall={handleCancelInstall}
        />
      )}

      {/* Git preview / selection dialog */}
      {gitPreview && (
        <GitPreviewDialog
          selections={gitSelections}
          setSelections={setGitSelections}
          confirmLoading={gitConfirmLoading}
          onClose={handleGitPreviewClose}
          onConfirm={handleGitConfirm}
        />
      )}
    </div>
  );
}
