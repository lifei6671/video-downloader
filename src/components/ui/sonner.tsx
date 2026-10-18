import { Toaster as Sonner, type ToasterProps } from "sonner"
import { CircleCheckIcon, InfoIcon, TriangleAlertIcon, OctagonXIcon, Loader2Icon } from "lucide-react"

const Toaster = ({ ...props }: ToasterProps) => {

  return (
    <Sonner
      theme="light"
      richColors
      closeButton
      containerAriaLabel="通知"
      className="toaster group"
      icons={{
        success: (
          <CircleCheckIcon className="size-4" />
        ),
        info: (
          <InfoIcon className="size-4" />
        ),
        warning: (
          <TriangleAlertIcon className="size-4" />
        ),
        error: (
          <OctagonXIcon className="size-4" />
        ),
        loading: (
          <Loader2Icon className="size-4 animate-spin" />
        ),
      }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
          "--success-bg": "color-mix(in srgb, var(--success) 8%, var(--popover))",
          "--success-border": "color-mix(in srgb, var(--success) 25%, var(--popover))",
          "--success-text": "var(--success)",
          "--error-bg": "color-mix(in srgb, var(--destructive) 8%, var(--popover))",
          "--error-border": "color-mix(in srgb, var(--destructive) 25%, var(--popover))",
          "--error-text": "var(--destructive)",
          "--warning-bg": "color-mix(in srgb, var(--warning) 8%, var(--popover))",
          "--warning-border": "color-mix(in srgb, var(--warning) 25%, var(--popover))",
          "--warning-text": "var(--warning)",
          "--info-bg": "var(--accent)",
          "--info-border": "color-mix(in srgb, var(--primary) 25%, var(--popover))",
          "--info-text": "var(--accent-foreground)",
        } as React.CSSProperties
      }
      toastOptions={{
        closeButtonAriaLabel: "关闭通知",
        classNames: {
          toast: "cn-toast",
        },
      }}
      {...props}
    />
  )
}

export { Toaster }
