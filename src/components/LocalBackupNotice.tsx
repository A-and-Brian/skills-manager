import { Monitor } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useApp } from "../context/AppContext";
import { cn } from "../utils";

/** Backup never follows the host switch; say so while a host is active. */
export function LocalBackupNotice({ className }: { className?: string }) {
  const { t } = useTranslation();
  const { activeHost } = useApp();
  if (!activeHost) return null;
  return (
    <div className={cn("app-panel flex items-start gap-2.5 px-4 py-3 text-[13px] leading-5 text-tertiary", className)}>
      <Monitor className="mt-0.5 h-4 w-4 shrink-0 text-accent" />
      <p>
        <span className="font-medium text-secondary">{t("remoteSession.backupLocalTitle")}</span>{" "}
        {t("remoteSession.backupLocalBody", { name: activeHost.name })}
      </p>
    </div>
  );
}
