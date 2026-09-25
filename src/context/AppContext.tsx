/* eslint-disable react-refresh/only-export-components */
import { createContext, useContext, useState, useEffect, useCallback, useRef, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import type { AppUpdateInfo, HostSessionInfo, ManagedSkill, Project, Preset, RemoteHost, ToolInfo } from "../lib/tauri";
import * as api from "../lib/tauri";
import { getActiveHostId, setActiveHostId, trackHost } from "../lib/hostCall";
import { listenOnActiveHost } from "../lib/hostEvents";
import { getErrorMessage } from "../lib/error";
import i18n from "../i18n";
import { applyTextSize } from "../lib/textScale";
import { useNavigate } from "@tanstack/react-router";
import { settingsLink } from "../views/settings/categories";
import { toast } from "sonner";

/** The live link to the active remote host. */
export interface HostSession {
  info: HostSessionInfo;
  /** Set when the link dropped; the next call or Reconnect connects again. */
  lostMessage: string | null;
}

interface AppState {
  presets: Preset[];
  /** Backend-tracked "last applied to default targets". Drives the "Applied to..." status, not the sidebar selection. */
  activePreset: Preset | null;
  /** Frontend-only "currently being viewed/edited" preset. Persisted to localStorage. UI selection. */
  viewedPreset: Preset | null;
  tools: ToolInfo[];
  managedSkills: ManagedSkill[];
  projects: Project[];
  remoteHosts: RemoteHost[];
  /** The remote host the app operates on; null is this computer. Never
   *  remembered across launches: the app always starts on this computer. */
  activeHost: RemoteHost | null;
  hostSession: HostSession | null;
  /** The host a switch is connecting to, while it connects. */
  connectingHostId: string | null;
  /** Operate on `hostId`, or on this computer for null. Resolves false when
   *  the host could not be reached; the app is then on this computer, or
   *  still on the host with its link marked lost when reconnecting. */
  switchHost: (hostId: string | null) => Promise<boolean>;
  /** Start the active host's session again, e.g. so it picks up a new
   *  library path. Resolves like `switchHost`. */
  reconnectHost: () => Promise<boolean>;
  loading: boolean;
  appError: string | null;
  helpOpen: boolean;
  detailSkillId: string | null;
  /** Result of the last app-version check. Notification only: installing an
   *  update is always started by the user from Settings. */
  appUpdate: AppUpdateInfo | null;
  refreshAppUpdate: () => Promise<AppUpdateInfo>;
  refreshAppData: () => Promise<void>;
  refreshPresets: () => Promise<void>;
  refreshTools: () => Promise<void>;
  refreshManagedSkills: () => Promise<void>;
  refreshProjects: () => Promise<void>;
  refreshRemoteHosts: () => Promise<void>;
  setViewedPresetId: (id: string) => void;
  applyPresetToDefault: (id: string) => Promise<void>;
  clearAppError: () => void;
  openHelp: () => void;
  closeHelp: () => void;
  openSkillDetailById: (skillId: string) => void;
  closeSkillDetail: () => void;
}

const VIEWED_PRESET_LS_KEY = "skills-manager.viewedPresetId";
const LEGACY_VIEWED_PRESET_LS_KEY = "skills-manager.viewedScenarioId";

/** Preset ids belong to one machine's library, so each host keeps its own. */
function readViewedPresetId(hostId: string | null): string | null {
  try {
    if (hostId) return localStorage.getItem(`${VIEWED_PRESET_LS_KEY}:${hostId}`);
    return localStorage.getItem(VIEWED_PRESET_LS_KEY) || localStorage.getItem(LEGACY_VIEWED_PRESET_LS_KEY);
  } catch {
    return null;
  }
}

function storeViewedPresetId(id: string) {
  const hostId = getActiveHostId();
  try {
    localStorage.setItem(hostId ? `${VIEWED_PRESET_LS_KEY}:${hostId}` : VIEWED_PRESET_LS_KEY, id);
  } catch {
    // localStorage may be unavailable; selection is still tracked in memory.
  }
}

const AppContext = createContext<AppState | null>(null);

export function AppProvider({ children }: { children: ReactNode }) {
  const SKILL_UPDATE_TOAST_ID = "skill-update-available";
  const APP_UPDATE_TOAST_ID = "app-update-available";
  const navigate = useNavigate();
  const [presets, setPresets] = useState<Preset[]>([]);
  const [activePreset, setActivePreset] = useState<Preset | null>(null);
  const [viewedPresetId, setViewedPresetIdState] = useState<string | null>(() => readViewedPresetId(null));
  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [managedSkills, setManagedSkills] = useState<ManagedSkill[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [remoteHosts, setRemoteHosts] = useState<RemoteHost[]>([]);
  const [activeHostId, setActiveHostIdState] = useState<string | null>(null);
  const [hostSession, setHostSession] = useState<HostSession | null>(null);
  const [connectingHostId, setConnectingHostId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [appError, setAppError] = useState<string | null>(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const [detailSkillId, setDetailSkillId] = useState<string | null>(null);
  const [appUpdate, setAppUpdate] = useState<AppUpdateInfo | null>(null);
  const autoCheckInFlightRef = useRef(false);
  const appUpdateCheckedRef = useRef(false);
  const lastUpdateNotificationRef = useRef<string | null>(null);
  const lastActivePresetIdRef = useRef<string | null>(null);
  const switchSeqRef = useRef(0);

  const setTranslatedError = useCallback((key: string) => {
    setAppError(i18n.t("common.loadFailed", { item: i18n.t(key) }));
  }, []);

  const refreshPresets = useCallback(async () => {
    const onSameHost = trackHost();
    try {
      const [s, active] = await Promise.all([
        api.getPresets(),
        api.getActivePreset(),
      ]);
      if (!onSameHost()) return;
      setPresets(s);
      setActivePreset(active);
      const previousActiveId = lastActivePresetIdRef.current;
      const nextActiveId = active?.id ?? null;
      if (previousActiveId !== nextActiveId) {
        lastActivePresetIdRef.current = nextActiveId;
        // Carry the sidebar along only when the user was viewing the old
        // active preset — that way an external switch (e.g. CLI) follows,
        // but a user who's browsing some other preset isn't yanked away.
        // Skip the initial load (previousActiveId === null) entirely so a
        // persisted viewedPreset from localStorage isn't clobbered.
        if (nextActiveId && previousActiveId !== null) {
          setViewedPresetIdState((current) => {
            if (current !== previousActiveId) return current;
            storeViewedPresetId(nextActiveId);
            return nextActiveId;
          });
        }
      }
      setAppError(null);
    } catch (e) {
      if (!onSameHost()) return;
      console.error("Failed to load presets:", e);
      setTranslatedError("common.presets");
    }
  }, [setTranslatedError]);

  const refreshTools = useCallback(async () => {
    const onSameHost = trackHost();
    try {
      const t = await api.getToolStatus();
      if (!onSameHost()) return;
      setTools(t);
      setAppError(null);
    } catch (e) {
      if (!onSameHost()) return;
      console.error("Failed to load tools:", e);
      setTranslatedError("common.agents");
    }
  }, [setTranslatedError]);

  const refreshProjects = useCallback(async () => {
    const onSameHost = trackHost();
    try {
      const p = await api.getProjects();
      if (!onSameHost()) return;
      setProjects(p);
    } catch (e) {
      if (!onSameHost()) return;
      console.error("Failed to load projects:", e);
    }
  }, []);

  const refreshRemoteHosts = useCallback(async () => {
    try {
      setRemoteHosts(await api.remoteHostsList());
    } catch (e) {
      console.error("Failed to load remote hosts:", e);
    }
  }, []);

  const refreshManagedSkills = useCallback(async () => {
    const onSameHost = trackHost();
    try {
      const skills = await api.getManagedSkills();
      if (!onSameHost()) return;
      setManagedSkills(skills);
      setAppError(null);
    } catch (e) {
      if (!onSameHost()) return;
      console.error("Failed to load managed skills:", e);
      setTranslatedError("common.skills");
    }
    // Managed skill changes affect project sync health badges
    refreshProjects();
  }, [setTranslatedError, refreshProjects]);

  const refreshAppData = useCallback(async () => {
    setLoading(true);
    await Promise.all([refreshPresets(), refreshTools(), refreshManagedSkills(), refreshProjects(), refreshRemoteHosts()]);
    setLoading(false);
  }, [refreshManagedSkills, refreshProjects, refreshPresets, refreshTools, refreshRemoteHosts]);

  const setViewedPresetId = useCallback((id: string) => {
    setViewedPresetIdState(id);
    storeViewedPresetId(id);
  }, []);

  /** Point every host-scoped call at `hostId` and drop the other machine's data. */
  const enterHost = useCallback((hostId: string | null) => {
    setActiveHostId(hostId);
    setActiveHostIdState(hostId);
    setViewedPresetIdState(readViewedPresetId(hostId));
    lastActivePresetIdRef.current = null;
    setPresets([]);
    setActivePreset(null);
    setTools([]);
    setManagedSkills([]);
    setProjects([]);
    setDetailSkillId(null);
    setAppError(null);
  }, []);

  const switchHost = useCallback(
    async (hostId: string | null): Promise<boolean> => {
      // Only the latest switch decides where the app ends up.
      const seq = ++switchSeqRef.current;
      const isLatest = () => switchSeqRef.current === seq;
      const previous = getActiveHostId();
      if (hostId === null) {
        setConnectingHostId(null);
        // Also abandons a connect in flight. This computer's data does not
        // wait for the host's session to close.
        api.remoteHostDisconnect().catch((e) => console.error("Failed to disconnect:", e));
        if (previous === null) return true;
        enterHost(null);
        setHostSession(null);
        await refreshAppData();
        return true;
      }
      setConnectingHostId(hostId);
      try {
        const info = await api.remoteHostConnect(hostId);
        if (!isLatest()) return false;
        if (previous !== hostId) enterHost(hostId);
        setHostSession({ info, lostMessage: null });
        await refreshAppData();
        return true;
      } catch (e) {
        if (!isLatest()) return false;
        const message = getErrorMessage(e, i18n.t("common.error"));
        const name = remoteHosts.find((h) => h.id === hostId)?.name ?? hostId;
        if (previous === hostId) {
          // Reconnecting: stay on the host with its link marked lost.
          setHostSession((s) => s && { ...s, lostMessage: message });
        } else if (previous !== null) {
          // Connecting closed the previous host's session.
          enterHost(null);
          setHostSession(null);
          await refreshAppData();
        }
        toast.error(i18n.t("hostSwitcher.connectFailed", { name }), {
          description: message,
          duration: 10000,
          action: { label: i18n.t("common.retry"), onClick: () => void switchHost(hostId) },
        });
        return false;
      } finally {
        if (isLatest()) setConnectingHostId(null);
      }
    },
    [enterHost, refreshAppData, remoteHosts]
  );

  const reconnectHost = useCallback(async () => {
    const hostId = getActiveHostId();
    if (hostId === null) return false;
    await api.remoteHostDisconnect().catch((e) => console.error("Failed to disconnect:", e));
    return switchHost(hostId);
  }, [switchHost]);

  // The app always starts on this computer (also after a webview reload,
  // which leaves the previous page's session open).
  useEffect(() => {
    api.remoteHostDisconnect().catch((e) => console.error("Failed to disconnect:", e));
  }, []);

  const handleApplyPresetToDefault = useCallback(
    async (id: string) => {
      await api.applyPresetToDefault(id);
      await Promise.all([refreshPresets(), refreshManagedSkills()]);
    },
    [refreshManagedSkills, refreshPresets]
  );

  // Resolve viewedPreset: persisted id > activePreset > first preset.
  // Persist whichever resolves so the next launch matches what the user saw.
  const viewedPreset = (() => {
    if (viewedPresetId) {
      const found = presets.find((s) => s.id === viewedPresetId);
      if (found) return found;
    }
    return activePreset ?? presets[0] ?? null;
  })();

  useEffect(() => {
    if (!viewedPreset) return;
    if (viewedPreset.id !== viewedPresetId) {
      // Persist the resolved fallback so subsequent reads are stable.
      setViewedPresetIdState(viewedPreset.id);
      storeViewedPresetId(viewedPreset.id);
    }
  }, [viewedPreset, viewedPresetId]);

  useEffect(() => {
    async function init() {
      // Both events log performance.now() (ms since timeOrigin) so the
      // reader can compute duration as done - start. Keeping the unit
      // identical to the other frontend startup marks avoids ambiguity in
      // the log file (see codex review note on #153).
      api.logStartupEvent("refresh_app_data_start", performance.now()).catch(() => {});
      await refreshAppData();
      api.logStartupEvent("refresh_app_data_done", performance.now()).catch(() => {});
      // Apply saved text size on startup
      const savedSize = await api.getSettings("text_size").catch(() => null);
      if (savedSize) {
        applyTextSize(savedSize);
      }
    }
    init();
  }, [refreshAppData]);

  useEffect(() => {
    const unlistenPromise = listen("tray-open-updates", () => {
      // The tray checked this computer's skills.
      void switchHost(null);
      setDetailSkillId(null);
      navigate({ to: "/my-skills" });
    });

    return () => {
      unlistenPromise
        .then((unlisten) => unlisten())
        .catch((error) => {
          console.error("Failed to unlisten tray-open-updates:", error);
        });
    };
  }, [navigate, switchHost]);

  useEffect(() => {
    let refreshTimer: ReturnType<typeof setTimeout> | null = null;

    const unlistenPromise = listenOnActiveHost("app-files-changed", () => {
      if (refreshTimer) {
        clearTimeout(refreshTimer);
      }
      refreshTimer = setTimeout(() => {
        refreshAppData().catch((error) => {
          console.error("Failed to refresh after filesystem change:", error);
        });
      }, 500);
    });

    return () => {
      if (refreshTimer) {
        clearTimeout(refreshTimer);
      }
      unlistenPromise
        .then((unlisten) => unlisten())
        .catch((error) => {
          console.error("Failed to unlisten app-files-changed:", error);
        });
    };
  }, [refreshAppData]);

  // The active host's link dropped (say so and offer Reconnect), or a call
  // connected it again.
  useEffect(() => {
    type SessionState = { host_id: string; state: string; message?: string };
    const unlistenPromise = listen<SessionState>("remote-session-state", ({ payload }) => {
      if (payload.host_id !== getActiveHostId()) return;
      const lostMessage = payload.state === "disconnected" ? payload.message ?? "" : null;
      setHostSession((s) => s && { ...s, lostMessage });
    });
    return () => {
      unlistenPromise
        .then((unlisten) => unlisten())
        .catch((error) => {
          console.error("Failed to unlisten remote-session-state:", error);
        });
    };
  }, []);

  const notifyUpdatableSkills = useCallback((skills: ManagedSkill[]) => {
    const updatable = skills
      .filter((s) => s.update_status === "update_available")
      .sort((a, b) => a.id.localeCompare(b.id));

    if (updatable.length === 0) {
      lastUpdateNotificationRef.current = null;
      toast.dismiss(SKILL_UPDATE_TOAST_ID);
      return;
    }

    const notificationSignature = updatable.map((skill) => skill.id).join("|");
    if (lastUpdateNotificationRef.current === notificationSignature) {
      return;
    }

    lastUpdateNotificationRef.current = notificationSignature;
    toast.info(
      i18n.t("mySkills.updateNotification", { count: updatable.length }),
      {
        id: SKILL_UPDATE_TOAST_ID,
        duration: 8000,
        action: {
          label: i18n.t("mySkills.viewUpdates"),
          onClick: () => {
            setDetailSkillId(null);
            navigate({ to: "/my-skills" });
          },
        },
      }
    );
  }, [navigate]);

  const refreshAppUpdate = useCallback(async () => {
    const info = await api.checkAppUpdate();
    setAppUpdate(info);
    return info;
  }, []);

  // Check for a newer app version on startup. This only ever *notifies* — the
  // download and install stay behind the button in Settings, so the user
  // decides whether to take an update. Deliberately unlike the skill
  // auto-update above, which has an opt-in "apply automatically" setting.
  //
  // Failures are logged, never toasted: this runs unprompted on every launch,
  // and users who cannot reach GitHub would otherwise get an error every time
  // they open the app.
  //
  // The ref makes it once per process, not once per `loading` edge:
  // `refreshAppData` flips `loading` on every call, and a file-change event or
  // a manual reload would otherwise re-hit the GitHub API and re-raise the
  // toast. An in-flight guard would not be enough — it only blocks overlap.
  //
  // Set inside the timer, not before it: `loading` flipping back to true within
  // the delay (the file watcher emits a change event as it builds its initial
  // watch set) tears this effect down and clears the pending timer, and marking
  // it done up front would skip the check for the rest of the session.
  useEffect(() => {
    if (loading || appUpdateCheckedRef.current) return;
    const timer = setTimeout(() => {
      appUpdateCheckedRef.current = true;
      refreshAppUpdate()
        .then((info) => {
          if (!info.has_update) return;
          toast.info(
            i18n.t("settings.updateAvailable", { version: info.latest_version }),
            {
              id: APP_UPDATE_TOAST_ID,
              duration: 8000,
              action: {
                label: i18n.t("settings.viewUpdate"),
                onClick: () => navigate(settingsLink("about")),
              },
            }
          );
        })
        .catch((err) => {
          console.error("Startup app update check failed:", err);
        });
    }, 3000);
    return () => clearTimeout(timer);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading]);

  // Check skill updates on startup (non-blocking, silent). When the user has
  // opted in via the Settings toggle, also apply any available updates.
  useEffect(() => {
    // The round is this computer's: a host runs its own from its own app.
    if (loading || activeHostId !== null || managedSkills.length === 0) return;
    const hasGitSkills = managedSkills.some(
      (s) => s.source_type === "git" || s.source_type === "skillssh"
    );
    if (!hasGitSkills || autoCheckInFlightRef.current) return;

    // Delay to avoid slowing down initial render
    const timer = setTimeout(() => {
      autoCheckInFlightRef.current = true;
      (async () => {
        try {
          await api.checkAllSkillUpdates(false);
          // Switched to a host meanwhile: the calls below would reach it.
          if (getActiveHostId() !== null) return;
          let skills = await api.getManagedSkills();

          const autoUpdate = await api
            .getSettings("auto_update_apply")
            .catch(() => null);
          if (autoUpdate === "on" && getActiveHostId() === null) {
            const ids = skills
              .filter(
                (s) =>
                  s.update_status === "update_available" &&
                  (s.source_type === "git" || s.source_type === "skillssh")
              )
              .map((s) => s.id);
            if (ids.length > 0) {
              const result = await api.batchUpdateSkills(ids);
              skills = await api.getManagedSkills();
              if (result.refreshed > 0) {
                toast.success(
                  i18n.t("mySkills.autoUpdated", { count: result.refreshed })
                );
              }
              // Held back rather than applied: updating would have removed
              // files the new version does not have, and nobody was here to ask.
              if (result.held_back.length > 0) {
                toast.warning(
                  i18n.t("mySkills.batchHeldBack", {
                    count: result.held_back.length,
                    names: result.held_back.slice(0, 3).join("、"),
                  })
                );
              }
              if (result.failed.length > 0) {
                console.warn("Auto-update failures:", result.failed);
                toast.error(
                  i18n.t("mySkills.autoUpdateFailed", {
                    count: result.failed.length,
                  })
                );
              }
            }
          }

          if (getActiveHostId() !== null) return;
          setManagedSkills(skills);
          notifyUpdatableSkills(skills);
          api.setSettings("auto_update_last_run_at", new Date().toISOString())
            .catch(() => {});
        } catch (err) {
          // Startup round is non-blocking and does not toast on failure, but
          // log so a broken check/update is still diagnosable.
          console.error("Startup skill update round failed:", err);
        } finally {
          autoCheckInFlightRef.current = false;
        }
      })();
    }, 3000);
    return () => clearTimeout(timer);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading]);

  // Refresh after a background auto-update round (Rust scheduler) or the
  // tray "check for updates" action finishes.
  useEffect(() => {
    const unlistenPromise = listenOnActiveHost("skills-auto-updated", async () => {
      try {
        const skills = await api.getManagedSkills();
        setManagedSkills(skills);
        notifyUpdatableSkills(skills);
      } catch (error) {
        console.error("Failed to refresh after skills-auto-updated:", error);
      }
    });
    return () => {
      unlistenPromise
        .then((unlisten) => unlisten())
        .catch((error) => {
          console.error("Failed to unlisten skills-auto-updated:", error);
        });
    };
  }, [notifyUpdatableSkills]);

  return (
    <AppContext.Provider
      value={{
        presets,
        activePreset,
        viewedPreset,
        tools,
        managedSkills,
        projects,
        remoteHosts,
        activeHost: remoteHosts.find((host) => host.id === activeHostId) ?? null,
        hostSession,
        connectingHostId,
        switchHost,
        reconnectHost,
        loading,
        appError,
        helpOpen,
        detailSkillId,
        appUpdate,
        refreshAppUpdate,
        refreshAppData,
        refreshPresets,
        refreshTools,
        refreshManagedSkills,
        refreshProjects,
        refreshRemoteHosts,
        setViewedPresetId,
        applyPresetToDefault: handleApplyPresetToDefault,
        clearAppError: () => setAppError(null),
        openHelp: () => setHelpOpen(true),
        closeHelp: () => setHelpOpen(false),
        openSkillDetailById: (skillId: string) => setDetailSkillId(skillId),
        closeSkillDetail: () => setDetailSkillId(null),
      }}
    >
      {children}
    </AppContext.Provider>
  );
}

export function useApp() {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within AppProvider");
  return ctx;
}
