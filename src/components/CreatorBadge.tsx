import { useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { cn } from "../utils";
import type { SkillCreator } from "../lib/skillCreator";

interface CreatorBadgeProps {
  creator: SkillCreator;
  size?: "sm" | "md";
  /** Off inside other buttons (filter pills), where a nested link can't live. */
  linked?: boolean;
  /** Dense rows and cards render nothing for a local skill rather than "local · Local". */
  hideLocal?: boolean;
  className?: string;
}

/**
 * Who made a skill: GitHub avatar and `@owner` linking to the repo, `@owner`
 * for other git hosts, the frontmatter author as plain text, or "Local".
 */
export function CreatorBadge({
  creator,
  size = "sm",
  linked = true,
  hideLocal = false,
  className,
}: CreatorBadgeProps) {
  const { t } = useTranslation();
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  const textSize = size === "sm" ? "text-[12px]" : "text-[13px]";

  if (creator.kind === "local" && hideLocal) return null;
  if (creator.kind === "local" || creator.kind === "author") {
    const label = creator.kind === "author" ? creator.name : t("mySkills.creator.local");
    return (
      <span className={cn("min-w-0 truncate text-muted", textSize, className)} title={label}>
        {label}
      </span>
    );
  }

  const src = creator.kind === "github" ? `https://github.com/${creator.owner}.png?size=32` : null;
  const avatarSize = size === "sm" ? "h-3.5 w-3.5 text-[8px]" : "h-4 w-4 text-[9px]";
  const content = (
    <>
      {src && src !== failedSrc ? (
        <img
          src={src}
          alt=""
          loading="lazy"
          draggable={false}
          className={cn("shrink-0 rounded-full border border-border-subtle", avatarSize)}
          onError={() => setFailedSrc(src)}
        />
      ) : (
        <span
          aria-hidden="true"
          className={cn(
            "inline-flex shrink-0 items-center justify-center rounded-full bg-surface-active font-semibold text-muted",
            avatarSize
          )}
        >
          {creator.owner.charAt(0).toUpperCase()}
        </span>
      )}
      <span className="truncate">@{creator.owner}</span>
    </>
  );
  const classes = cn("inline-flex min-w-0 items-center gap-1 text-muted", textSize, className);

  if (!linked) return <span className={classes}>{content}</span>;

  const host = creator.kind === "github" ? "github.com" : creator.host;
  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        openUrl(creator.url).catch(() => {});
      }}
      // Rows that open on Enter or Space must not also open when the link is activated.
      onKeyDown={(e) => e.stopPropagation()}
      title={t("mySkills.creator.openRepo", { repo: `${creator.owner}/${creator.repo}`, host })}
      className={cn(classes, "rounded outline-none transition-colors hover:text-secondary focus-visible:ring-2 focus-visible:ring-border")}
    >
      {content}
    </button>
  );
}
