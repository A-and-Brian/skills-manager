import { useTranslation } from "react-i18next";
import { cn } from "../utils";
import type { ProjectDeployMode } from "../lib/tauri";

interface Props {
  value: ProjectDeployMode;
  onChange: (mode: ProjectDeployMode) => void;
  disabled?: boolean;
  className?: string;
}

/** "Linked to library" or "Vendored copy", with a line on what the current choice means. */
export function DeployModePicker({ value, onChange, disabled, className }: Props) {
  const { t } = useTranslation();
  return (
    <div className={className}>
      <div className="app-segmented flex w-full">
        {(["link", "copy"] as const).map((mode) => (
          <button
            key={mode}
            type="button"
            onClick={() => onChange(mode)}
            disabled={disabled}
            className={cn("app-segmented-button flex-1", value === mode && "app-segmented-button-active")}
          >
            {t(`project.settings.mode.${mode}`)}
          </button>
        ))}
      </div>
      <p className="mt-1.5 text-[12px] text-muted">{t(`project.settings.modeHint.${value}`)}</p>
    </div>
  );
}
