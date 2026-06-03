import type { HTMLAttributes } from "react";

import { cn } from "../../lib/utils";

type BadgeVariant = "default" | "secondary" | "success" | "destructive" | "info";

const variants: Record<BadgeVariant, string> = {
  default: "bg-primary text-primary-foreground",
  secondary: "bg-muted text-foreground",
  success: "bg-emerald-700 text-white",
  destructive: "bg-destructive text-destructive-foreground",
  info: "bg-blue-600 text-white",
};

export function Badge({
  className,
  variant = "default",
  ...props
}: HTMLAttributes<HTMLSpanElement> & { variant?: BadgeVariant }) {
  return (
    <span
      className={cn(
        "inline-flex h-6 items-center rounded-md px-2 text-xs font-medium",
        variants[variant],
        className,
      )}
      {...props}
    />
  );
}
