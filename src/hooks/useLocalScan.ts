import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import * as api from "../lib/tauri";
import type { ScanResult } from "../lib/tauri";
import { getErrorMessage } from "../lib/error";

/**
 * Skills found in other agents' folders. The first scan runs once `active`;
 * later ones on demand, loudly (`runScan`) or silently after an install
 * (`runScanSilent`).
 */
export function useLocalScan(active: boolean) {
  const { t } = useTranslation();
  const [scanResult, setScanResult] = useState<ScanResult | null>(null);
  const [scanLoading, setScanLoading] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  const runScan = useCallback(async () => {
    setScanLoading(true);
    setLocalError(null);
    try {
      const result = await api.scanLocalSkills();
      setScanResult(result);
    } catch (error: unknown) {
      console.error(error);
      const message = getErrorMessage(error, t("common.error"));
      setLocalError(message);
      toast.error(message);
    } finally {
      setScanLoading(false);
    }
  }, [t]);

  // Silent variant used after install/import. Never surfaces a toast or
  // new error state — failure here must not mask the install success.
  // Clears any stale localError on success so successful operations don't
  // leave previous error banners behind.
  const runScanSilent = useCallback(async () => {
    try {
      const result = await api.scanLocalSkills();
      setScanResult(result);
      setLocalError(null);
    } catch (error: unknown) {
      console.warn("silent scan failed:", error);
    }
  }, []);

  useEffect(() => {
    if (active && !scanResult && !scanLoading) {
      runScan();
    }
  }, [active, scanLoading, scanResult, runScan]);

  return {
    scanResult,
    scanLoading,
    localError,
    setLocalError,
    runScan,
    runScanSilent,
  };
}
