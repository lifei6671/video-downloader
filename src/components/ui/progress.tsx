import * as React from "react"
import { cn } from "cn"
import { Progress as ProgressPrimitive } from "radix-ui"

function Progress({
  className,
  value,
  variant = "linear",
  updateIntervalMs,
  children,
  ...props
}: React.ComponentProps<typeof ProgressPrimitive.Root> & { variant?: "linear" | "circular"; updateIntervalMs?: number }) {
  const circular = variant === "circular"
  const previousValue = React.useRef(value)
  const reset = value == null || previousValue.current == null || value < previousValue.current
  React.useEffect(() => {
    previousValue.current = value
  }, [value])
  const percent = Math.max(0, Math.min(100, value ?? 0))
  const duration = reset ? 0 : updateIntervalMs ?? (circular ? 250 : 0)
  return (
    <ProgressPrimitive.Root
      value={value}
      data-slot="progress"
      data-variant={variant}
      className={cn(
        circular ? "progress-circular" : "relative flex h-1 w-full items-center overflow-x-hidden rounded-full bg-muted",
        className
      )}
      {...props}
    >
      <ProgressPrimitive.Indicator
        data-slot="progress-indicator"
        className={circular ? "progress-circular-indicator" : "size-full flex-1 bg-primary transition-all"}
        style={{
          ...(circular ? { "--progress-percentage": percent } : { transform: `translateX(-${100 - percent}%)` }),
          transitionProperty: circular ? "--progress-percentage" : "transform",
          transitionDuration: `${duration}ms`,
          transitionTimingFunction: "linear",
        } as React.CSSProperties}
      />
      {circular && <span className="progress-circular-label" aria-hidden="true">{children ?? (value == null ? "—" : `${Math.round(percent)}%`)}</span>}
    </ProgressPrimitive.Root>
  )
}

export { Progress }
