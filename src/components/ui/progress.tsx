import * as React from "react"
import { cn } from "cn"
import { Progress as ProgressPrimitive } from "radix-ui"

function Progress({
  className,
  value,
  variant = "linear",
  children,
  ...props
}: React.ComponentProps<typeof ProgressPrimitive.Root> & { variant?: "linear" | "circular" }) {
  const circular = variant === "circular"
  const [animatedValue, setAnimatedValue] = React.useState(value ?? 0)
  const currentValue = React.useRef(value ?? 0)
  const previousValue = React.useRef(value)
  React.useEffect(() => {
    if (!circular) return
    const previous = previousValue.current
    previousValue.current = value
    const target = value ?? 0
    const media = matchMedia("(prefers-reduced-motion: reduce)")
    if (value == null || previous == null || target < previous || target <= currentValue.current || media.matches) {
      currentValue.current = target
      setAnimatedValue(target)
      return
    }
    const start = currentValue.current
    const startedAt = performance.now()
    let frame = 0
    const tick = (now: number) => {
      const elapsed = media.matches ? 1 : Math.max(0, Math.min(1, (now - startedAt) / 250))
      const next = start + (target - start) * (1 - (1 - elapsed) ** 3)
      currentValue.current = next
      setAnimatedValue(next)
      if (elapsed < 1) frame = requestAnimationFrame(tick)
    }
    frame = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(frame)
  }, [circular, value])
  const visualValue = value == null ? 0 : previousValue.current == null || value < previousValue.current ? value : Math.max(0, Math.min(value, animatedValue))
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
        style={circular ? { background: `conic-gradient(var(--progress-value) ${Math.min(100, visualValue) * 3.6}deg, var(--progress-track) 0deg)` } : { transform: `translateX(-${100 - (value || 0)}%)` }}
      />
      {circular && <span className="progress-circular-label" aria-hidden="true">{children ?? (value == null ? "—" : `${Math.round(visualValue)}%`)}</span>}
    </ProgressPrimitive.Root>
  )
}

export { Progress }
