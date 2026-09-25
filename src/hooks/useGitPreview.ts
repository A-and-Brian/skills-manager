import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useApp } from "../context/AppContext";
import * as api from "../lib/tauri";
import type { GitPreviewResult } from "../lib/tauri";
import { listenOnActiveHost } from "../lib/hostEvents";
import { getErrorMessage, getErrorKind } from "../lib/error";

/** A skill found in a previewed repo, and whether and as what to install it. */
export type GitSelection = { rel_path: string; name: string; description: string | null; selected: boolean };

/**
 * Install from a git URL in two steps: clone and preview the repo's skills
 * (with clone progress on a toast), then install the selected ones.
 */
export function useGitPreview() {
  const { t } = useTranslation();
  const { refreshPresets, refreshManagedSkills } = useApp();
  const [gitUrl, setGitUrl] = useState("");
  const [gitLoading, setGitLoading] = useState(false);
  const [gitCancelKey, setGitCancelKey] = useState<string | null>(null);
  const [gitPreview, setGitPreview] = useState<GitPreviewResult | null>(null);
  const [gitPreviewRepoUrl, setGitPreviewRepoUrl] = useState<string | null>(null);
  const [gitSelections, setGitSelections] = useState<GitSelection[]>([]);
  const [gitConfirmLoading, setGitConfirmLoading] = useState(false);

  const handleGitPreview = async () => {
    if (!gitUrl.trim()) return;
    setGitLoading(true);
    const url = gitUrl.trim();
    setGitCancelKey(url);

    const toastId = toast.loading(t("install.toast.cloning"));
    let unlisten: (() => void) | null = null;

    try {
      unlisten = await listenOnActiveHost<{ skill_id: string; phase: string; detail?: string }>(
        "install-progress",
        (event) => {
          if (event.payload.skill_id !== url) return;
          if (event.payload.phase === "cloning") {
            const detail = event.payload.detail?.trim();
            const msg = detail
              ? `${t("install.toast.cloning")}\n${detail}`
              : t("install.toast.cloning");
            toast.loading(msg, { id: toastId });
          }
        }
      );
      const preview = await api.previewGitInstall(url);
      toast.dismiss(toastId);
      setGitPreview(preview);
      setGitPreviewRepoUrl(url);
      setGitSelections(preview.skills.map((s) => ({
        rel_path: s.rel_path,
        name: s.name,
        description: s.description,
        selected: true,
      })));
    } catch (error: unknown) {
      if (getErrorKind(error) === "cancelled") {
        toast.info(t("install.toast.cancelled"), { id: toastId });
      } else {
        toast.error(getErrorMessage(error, t("common.error")), { id: toastId });
      }
    } finally {
      setGitLoading(false);
      setGitCancelKey(null);
      unlisten?.();
    }
  };

  const handleGitPreviewClose = () => {
    if (gitConfirmLoading) return;
    if (gitPreview) {
      api.cancelGitPreview(gitPreview.temp_dir).catch(() => {});
    }
    setGitPreview(null);
    setGitPreviewRepoUrl(null);
    setGitSelections([]);
  };

  const handleGitConfirm = async () => {
    if (!gitPreview) return;
    const repoUrl = gitPreviewRepoUrl ?? gitUrl.trim();
    if (!repoUrl) return;
    const selected = gitSelections.filter((s) => s.selected);
    if (selected.length === 0) return;
    setGitConfirmLoading(true);
    try {
      await api.confirmGitInstall(
        repoUrl,
        gitPreview.temp_dir,
        selected.map((s) => ({ rel_path: s.rel_path, name: s.name }))
      );
      await Promise.all([refreshPresets(), refreshManagedSkills()]);
      toast.success(t("install.toast.success", { name: selected.map((s) => s.name).join(", ") }));
      setGitUrl("");
      setGitPreview(null);
      setGitPreviewRepoUrl(null);
      setGitSelections([]);
    } catch (error: unknown) {
      toast.error(getErrorMessage(error, t("common.error")));
    } finally {
      setGitConfirmLoading(false);
    }
  };

  return {
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
  };
}
