import type { ButtonHTMLAttributes, ReactNode } from 'react'

/* Gallop's Button and Spinner as @cowboyinc/gallop-components 0.7.0 renders them (dist/index.mjs), since the site
   cannot install the package from Cowboy's private registry. Each draws the package's own Tailwind classes on Gallop's
   tokens (gallop.css), so it looks and behaves as the package's does. Only what the page uses is here: the primary and
   secondary Button at sizes s and m with an icon before the label, and the default Spinner. */

const join = (...parts: (string | false | undefined)[]) => parts.filter(Boolean).join(' ')

const BUTTON_BASE = 'inline-flex items-center justify-center rounded-full cursor-pointer select-none transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-40'

const BUTTON_VARIANT = {
  primary: 'relative overflow-hidden bg-button-primary-default enabled:hover:bg-button-primary-hover enabled:active:bg-button-primary-pressed disabled:bg-button-primary-disabled text-button-primary-text-default enabled:hover:text-button-primary-text-hover enabled:active:text-button-primary-text-pressed btn-primary-glow btn-primary-text-shadow',
  secondary: 'bg-button-secondary-default enabled:hover:bg-button-secondary-hover enabled:active:bg-button-secondary-pressed disabled:bg-button-secondary-disabled text-button-secondary-text-default enabled:hover:text-button-secondary-text-hover enabled:active:text-button-secondary-text-pressed',
}

const BUTTON_SIZE = {
  m: 'px-6 py-3 gap-2 type-body-l-strong',
  s: 'px-4 py-2 gap-1.5 type-body-m-strong',
}

const ICON_SIZE = { m: 'size-6', s: 'size-5' }
const ICON_BEFORE_NUDGE = { m: '-ml-1', s: '-ml-0.5' }
const ICON_TONE = { primary: 'btn-primary-icon-shadow btn-primary-icon-color', secondary: 'btn-secondary-icon-color' }

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: keyof typeof BUTTON_VARIANT
  size?: keyof typeof BUTTON_SIZE
  iconBefore?: ReactNode
}

export function Button({ variant = 'primary', size = 'm', iconBefore, className, children, type = 'button', ...props }: ButtonProps) {
  return (
    <button type={type} className={join(BUTTON_BASE, BUTTON_VARIANT[variant], BUTTON_SIZE[size], className)} {...props}>
      {iconBefore && <span className={join('shrink-0', ICON_SIZE[size], ICON_BEFORE_NUDGE[size], ICON_TONE[variant])}>{iconBefore}</span>}
      {children}
    </button>
  )
}

// The package's default Spinner: a turning arc in Nugget/500, which a caller recolours.
export function Spinner({ className, label = 'Loading' }: { className?: string; label?: string }) {
  return (
    <span role="status" aria-label={label} className={join('inline-block size-3 text-nugget-500', className)}>
      <svg viewBox="0 0 12 12" fill="none" className="size-full animate-spin">
        <path d="M11 6C11 8.76142 8.76142 11 6 11C3.23858 11 1 8.76142 1 6C1 3.23858 3.23858 1 6 1" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      </svg>
    </span>
  )
}
