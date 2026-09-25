import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { cn } from "../../utils";
import * as api from "../../lib/tauri";
import { SEGMENTED_BUTTON_CLASS } from "./shared";

export function AutoUpdateSection() {
  const { t } = useTranslation();
  const [autoUpdateInterval, setAutoUpdateInterval] = useState("off");
  const [autoUpdateApply, setAutoUpdateApply] = useState("off");
  const [autoUpdateLastRun, setAutoUpdateLastRun] = useState<string | null>(null);

  useEffect(() => {
    api.getSettings("auto_update_check_interval").then((v) => { if (v) setAutoUpdateInterval(v); });
    api.getSettings("auto_update_apply").then((v) => { if (v) setAutoUpdateApply(v); });
    // The `skills-auto-updated` listener may populate this concurrently, so
    // keep whichever timestamp is newer rather than blindly overwriting.
    api.getSettings("auto_update_last_run_at").then((v) => {
      if (!v) return;
      setAutoUpdateLastRun((prev) =>
        prev && Date.parse(prev) >= Date.parse(v) ? prev : v
      );
    });
  }, []);

  const handleAutoUpdateIntervalChange = async (value: string) => {
    setAutoUpdateInterval(value);
    await api.setSettings("auto_update_check_interval", value);
  };

  const handleAutoUpdateApplyChange = async (value: string) => {
    setAutoUpdateApply(value);
    await api.setSettings("auto_update_apply", value);
  };

  // Keep the last-run timestamp in sync with both the background scheduler
  // and the tray's manual "Check for skill updates" so the user doesn't see
  // a stale value if Settings is open. Backend always persists `last_run_at`
  // first and then emits with the same `ran_at`, so reading from the payload
  // avoids a follow-up DB roundtrip.
  useEffect(() => {
    type AutoUpdatedPayload = { ran_at?: string };
    const unlistenPromise = listen<AutoUpdatedPayload>("skills-auto-updated", (event) => {
      const ranAt = event.payload?.ran_at;
      if (ranAt) {
        setAutoUpdateLastRun(ranAt);
      }
    });
    return () => {
      unlistenPromise
        .then((unlisten) => unlisten())
        .catch(() => {});
    };
  }, []);

  const autoUpdateIntervalOptions = [
    { value: "off", label: t("settings.autoUpdate.intervalOff") },
    { value: "1h", label: t("settings.autoUpdate.interval1h") },
    { value: "6h", label: t("settings.autoUpdate.interval6h") },
    { value: "24h", label: t("settings.autoUpdate.interval24h") },
  ] as const;
  const autoUpdateApplyOptions = [
    { value: "off", label: t("settings.autoUpdate.applyOff") },
    { value: "on", label: t("settings.autoUpdate.applyOn") },
  ] as const;

  return (
    <section>
      <h2 className="app-section-title mb-3">
        {t("settings.autoUpdate.title")}
      </h2>
      <div className="app-panel overflow-hidden divide-y divide-border-faint">
        <div className="flex items-center justify-between gap-4 px-4 py-2.5">
          <div className="min-w-0">
            <h3 className="text-[14px] font-semibold text-primary">
              {t("settings.autoUpdate.intervalLabel")}
            </h3>
            <p className="text-[12px] text-muted">
              {t("settings.autoUpdate.intervalDesc")}
              {autoUpdateLastRun
                ? ` · ${t("settings.autoUpdate.lastRun", {
                    time: new Date(autoUpdateLastRun).toLocaleString(),
                  })}`
                : ""}
            </p>
          </div>
          <div className="app-segmented flex-wrap bg-background">
            {autoUpdateIntervalOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                aria-pressed={autoUpdateInterval === option.value}
                onClick={() => handleAutoUpdateIntervalChange(option.value)}
                className={cn(
                  SEGMENTED_BUTTON_CLASS,
                  autoUpdateInterval === option.value
                    ? "bg-surface-active text-secondary"
                    : "text-muted hover:text-tertiary"
                )}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>
        <div className="flex items-center justify-between gap-4 px-4 py-2.5">
          <div className="min-w-0">
            <h3 className="text-[14px] font-semibold text-primary">
              {t("settings.autoUpdate.applyLabel")}
            </h3>
            <p className="text-[12px] text-muted">
              {t("settings.autoUpdate.applyDesc")}
            </p>
          </div>
          <div className="app-segmented flex-wrap bg-background">
            {autoUpdateApplyOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                aria-pressed={autoUpdateApply === option.value}
                onClick={() => handleAutoUpdateApplyChange(option.value)}
                className={cn(
                  SEGMENTED_BUTTON_CLASS,
                  autoUpdateApply === option.value
                    ? "bg-surface-active text-secondary"
                    : "text-muted hover:text-tertiary"
                )}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
