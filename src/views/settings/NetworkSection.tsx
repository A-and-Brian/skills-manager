import { useState, useEffect } from "react";
import { Link as LinkIcon, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import * as api from "../../lib/tauri";
import { ACTION_BUTTON_CLASS, FIELD_CLASS } from "./shared";
import { HostBadge } from "../../components/HostBadge";

export function NetworkSection() {
  const { t } = useTranslation();
  const [proxyInput, setProxyInput] = useState("");
  const [proxySaving, setProxySaving] = useState(false);

  useEffect(() => {
    api.getSettings("proxy_url").then((v) => { setProxyInput(v ?? ""); });
  }, []);

  const handleSaveProxy = async () => {
    const trimmed = proxyInput.trim();
    if (trimmed && !/^(https?|socks5):\/\//i.test(trimmed)) {
      toast.error(t("settings.proxyUrlInvalid"));
      return;
    }
    setProxySaving(true);
    try {
      await api.setSettings("proxy_url", trimmed);
      toast.success(t("settings.proxyUrlSaved"));
    } catch {
      toast.error(t("common.error"));
    } finally {
      setProxySaving(false);
    }
  };

  return (
    <section>
      <h2 className="app-section-title mb-3">
        {t("settings.proxyConfig")}
        <HostBadge />
      </h2>
      <div className="app-panel overflow-hidden divide-y divide-border-faint">
        <div className="px-4 py-3">
          <h3 className="text-[14px] font-semibold text-primary">{t("settings.proxyUrl")}</h3>
          <p className="mt-0.5 mb-2 text-[12px] text-muted">{t("settings.proxyUrlDesc")}</p>
          <div className="flex flex-wrap items-center gap-2">
            <input
              type="text"
              value={proxyInput}
              onChange={(e) => setProxyInput(e.target.value)}
              placeholder={t("settings.proxyUrlPlaceholder")}
              className={`${FIELD_CLASS} min-w-0 flex-1 font-mono`}
            />
            <button
              onClick={handleSaveProxy}
              disabled={proxySaving}
              className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
            >
              {proxySaving ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : (
                <LinkIcon className="w-3 h-3" />
              )}
              {t("common.save")}
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}
