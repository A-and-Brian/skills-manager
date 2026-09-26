import { useEffect } from "react";
import { Pencil, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

export interface TagMenuState {
  tag: string;
  x: number;
  y: number;
}

interface TagContextMenuProps {
  menu: TagMenuState;
  onClose: () => void;
  onRename: (tag: string) => void;
  onDelete: (tag: string) => void;
}

/** Right-click menu for a tag in the library filter popover (#233). */
export function TagContextMenu({ menu, onClose, onRename, onDelete }: TagContextMenuProps) {
  const { t } = useTranslation();

  // Close on Escape (click-outside is handled by the backdrop).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <>
      {/* Backdrop closes on left- or right-click outside the menu. Explicit
          z-index (z-40/z-50) to avoid the macOS WKWebView stacking bug. */}
      <div
        className="fixed inset-0 z-40"
        onClick={() => onClose()}
        onContextMenu={(e) => {
          e.preventDefault();
          onClose();
        }}
      />
      <div
        className="fixed z-50 min-w-[140px] overflow-hidden rounded-lg border border-border bg-surface py-1 shadow-2xl"
        style={{ top: menu.y, left: menu.x }}
      >
        <button
          onClick={() => {
            onRename(menu.tag);
            onClose();
          }}
          className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px] text-secondary hover:bg-surface-hover"
        >
          <Pencil className="h-3.5 w-3.5" />
          {t("mySkills.tags.renameTag")}
        </button>
        <button
          onClick={() => {
            onDelete(menu.tag);
            onClose();
          }}
          className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px] text-red-400 hover:bg-surface-hover"
        >
          <Trash2 className="h-3.5 w-3.5" />
          {t("mySkills.tags.deleteTag")}
        </button>
      </div>
    </>
  );
}
