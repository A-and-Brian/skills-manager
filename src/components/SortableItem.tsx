import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { cn } from "../utils";

/** Spread onto the element that should start a drag (the grip handle). */
export type SortableHandleProps = NonNullable<ReturnType<typeof useSortable>["listeners"]> & {
  ref: (element: HTMLElement | null) => void;
  onClick: (event: React.MouseEvent) => void;
};

interface SortableItemProps {
  id: string;
  disabled?: boolean;
  className?: string;
  children: (handleProps: SortableHandleProps) => React.ReactNode;
}

export function SortableItem({ id, disabled, className, children }: SortableItemProps) {
  const {
    attributes,
    listeners,
    setNodeRef,
    setActivatorNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id, disabled });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : undefined,
  };

  const handleProps: SortableHandleProps = {
    ...listeners,
    ref: setActivatorNodeRef,
    onClick: (e) => e.stopPropagation(),
  };

  return (
    <div ref={setNodeRef} style={style} {...attributes} className={cn("h-full", className)}>
      {children(handleProps)}
    </div>
  );
}
