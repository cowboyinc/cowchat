import type { AnchorHTMLAttributes, ReactNode } from 'react'
import './text-link.css'

/* TEXT LINK, copied from the prototype (cowboyinc/prototype-site, src/components/ui/TextLink.tsx): a plain text link in
   the page's quiet ink that comes up to primary under the pointer. Gallop has Button and Icon Button but no link, so
   this is local. `size`: m is Body/M for a bar's links, s is Body/S for a footer. `tone="on-dark"` is the same link on a
   dark ground (bison 300, hay 100 under the pointer). An address off the site opens in a new tab. */
export function TextLink({ href, size = 'm', tone, className = '', children, ...props }: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string; size?: 'm' | 's'; tone?: 'on-dark'; children: ReactNode }) {
  const external = /^https?:\/\//.test(href)
  return <a href={href} className={`text-link text-link-${size}${tone ? ` text-link-${tone}` : ''}${className ? ` ${className}` : ''}`} {...(external ? { target: '_blank', rel: 'noreferrer' } : {})} {...props}>{children}</a>
}
