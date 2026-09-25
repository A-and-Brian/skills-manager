import { useTranslation } from "react-i18next";
import { useApp } from "../context/AppContext";

/**
 * Why a native file picker or Finder action is unavailable, or undefined when
 * it works. They only see this computer's disk, so they are off while a remote
 * host is active.
 */
export function useRemotePickerBlock(): string | undefined {
  const { t } = useTranslation();
  const { activeHost } = useApp();
  return activeHost ? t("remoteSession.pickerUnavailable", { name: activeHost.name }) : undefined;
}
