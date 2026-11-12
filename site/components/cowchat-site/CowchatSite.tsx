'use client'
import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react'
import { Button } from './gallop'
import * as G from './icons'
import { HeroShot } from './shots/hero/HeroShot'
import { JoinShot } from './shots/join/JoinShot'
import { RoomShot } from './shots/room/RoomShot'
import { TeamShot } from './shots/team/TeamShot'
import { applyTheme, subscribeSystem } from '../theme'
import { TextLink } from './TextLink'
import { AGENTS, CS, DOWNLOAD, GITHUB, type CsAgent, type CsInstall, type CsJoin } from './copy'
import './shared.css'
import './cowchat-site.css'

/* Cowchat's marketing page at cowchat.cowboy.inc (site/README.md), designed in the prototype (cowboyinc/prototype-site,
   /cowchat-site, PR 531) and ported here section for section. Ten parts top to bottom: the bar, the hero over shot 1 on
   a band, three sections that each show one shot in their own form (words beside it, a heading over it, it beside
   words), the Works with card, the install rows, the open source band, the close and the footer. Every word is
   Codex's, from copy.ts. The four shots are drawn from Gallop as recreations of the Mac app's screens (shots/).

   It shares cowboy.inc's column, line, head and command row (.ss-wrap, .ss-line, .sf-head, .sf-section, .sf-code) and
   Marshal's site's frame (the page, the bar, the hero, the actions, the shot's drop, the band, the footer and the
   motion, the .ms- classes), copied into shared.css. Its own rules are cowchat-site.css. It follows the system's light
   or dark scheme (../theme.ts). Gallop's Button and icons are local copies (gallop.tsx, icons.tsx), since the site
   cannot install the packages. */

const ICON = '/assets/cowchat-site/cowchat-icon.png'
const reducedMotion = () => typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches

function AppIcon({ size, className = '' }: { size: number; className?: string }) {
  // The Mac app's icon (cowboyinc/cowchat, apps/CowchatMac/Sources/CowchatMac/Resources/CowchatIcon.png at 256), which
  // macOS masks to its rounded square; the page rounds it the same way.
  return <img className={`cs-icon ${className}`.trim()} src={ICON} width={size} height={size} alt="" />
}

// A product shot's frame: the window's ground and edge, holding one of the four shots drawn from Gallop as recreations
// of the Mac app's screens (shots/, plan, Product shots). Each shot's height is its own, set in cowchat-site.css.
function Shot({ id, label, children }: { id: 1 | 2 | 3 | 4; label: string; children: ReactNode }) {
  return <figure className="ms-shot cs-shot" aria-label={label} data-cs-shot={id}>{children}</figure>
}

// The agent's mark on a tile, as the other rows' glyphs sit: the owner's own file, unaltered (plan, Decisions 1;
// site/README.md records each file's source). An owner's dark file takes over in the dark theme.
function AgentMark({ agent }: { agent: CsAgent }) {
  return (
    <span className="cs-mark" data-cs-mark={agent.id} role="img" aria-label={agent.mark}>
      <img src={agent.src} alt="" data-cs-theme={agent.srcDark ? 'light' : undefined} />
      {agent.srcDark && <img src={agent.srcDark} alt="" data-cs-theme="dark" />}
    </span>
  )
}

function JoinArt({ id }: { id: CsJoin['id'] }) {
  if (id === 'agents') return <span className="cs-join-marks">{AGENTS.map((agent) => <AgentMark key={agent.id} agent={agent} />)}</span>
  return <span className="cs-join-glyph" aria-hidden="true">{id === 'shell' ? <G.Chatbot /> : <G.Posse />}</span>
}

function InstallRow({ row, copied, onCopy }: { row: CsInstall; copied: boolean; onCopy: () => void }) {
  const agent = row.id === 'plugin' ? AGENTS[0] : row.id === 'codex' ? AGENTS[1] : null
  return (
    <li className="cs-install-row" data-cs-install={row.id}>
      <div className="cs-install-head">
        {agent ? <AgentMark agent={agent} /> : <AppIcon size={32} className="cs-install-icon" />}
        <div>
          <h3>{row.title}</h3>
          <p>{row.line}</p>
        </div>
      </div>
      <div className="sf-code cs-code">
        <code>{row.command}</code>
        <span className="sf-code-actions">
          <Button variant="secondary" size="s" type="button" iconBefore={copied ? <G.Checkmark /> : <G.Copy />} onClick={onCopy}>
            {/* Both labels share one cell, so the button keeps the wider word's width when Copy turns to Copied. */}
            <span className="sf-copy-label"><span aria-hidden={copied || undefined} data-on={!copied || undefined}>{CS.install.copy}</span><span aria-hidden={!copied || undefined} data-on={copied || undefined}>{CS.install.copied}</span></span>
          </Button>
        </span>
      </div>
    </li>
  )
}

export function CowchatSite() {
  // Cowchat's page follows the system's scheme, as Marshal's does. The layout's script sets it before the first paint;
  // this keeps it in step when the system's scheme changes while the page is open (../theme.ts).
  useLayoutEffect(() => {
    applyTheme()
    return subscribeSystem(() => applyTheme())
  }, [])

  const [scrolled, setScrolled] = useState(false)
  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 8)
    onScroll()
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])

  // The bar's Download shows only once the hero's has scrolled under the bar, so the page never offers it twice at once
  // (Patrick, 2 November 2026, of Marshal's Request access).
  const heroActions = useRef<HTMLDivElement>(null)
  const [barDownload, setBarDownload] = useState(false)
  useEffect(() => {
    if (!heroActions.current) return
    const observer = new IntersectionObserver(([entry]) => setBarDownload(!entry.isIntersecting), { rootMargin: '-72px 0px 0px 0px' })
    observer.observe(heroActions.current)
    return () => observer.disconnect()
  }, [])

  // Content rises into place once as it enters the window, only when motion is allowed and the page has mounted, so a
  // reader without scripts, a reduced-motion reader and a screenshot see every section in place.
  const root = useRef<HTMLDivElement>(null)
  const [motion, setMotion] = useState(false)
  useEffect(() => {
    if (reducedMotion() || !root.current) return
    const targets = [...root.current.querySelectorAll<HTMLElement>('[data-reveal]')]
    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) if (entry.isIntersecting) { entry.target.setAttribute('data-in', ''); observer.unobserve(entry.target) }
    }, { rootMargin: '0px 0px -10% 0px' })
    for (const target of targets) if (target.getBoundingClientRect().top > window.innerHeight) observer.observe(target); else target.setAttribute('data-in', '')
    setMotion(true)
    return () => observer.disconnect()
  }, [])

  const [copied, setCopied] = useState<CsInstall['id'] | null>(null)
  // Copy reads Copied only once the clipboard holds the command. Where the browser refuses, the command is selected for
  // the reader to copy by hand and the button keeps reading Copy.
  const copy = (row: CsInstall) => () => {
    const command = row.command
    const written = navigator.clipboard ? navigator.clipboard.writeText(command) : Promise.reject(new Error('no clipboard'))
    written.then(() => {
      setCopied(row.id)
      window.setTimeout(() => setCopied((current) => (current === row.id ? null : current)), 1600)
    }, () => {
      const code = document.querySelector(`[data-cs-install="${row.id}"] .cs-code code`)
      if (code) window.getSelection()?.selectAllChildren(code)
    })
  }

  const go = (id: string) => (event: MouseEvent) => {
    event.preventDefault()
    document.getElementById(id)?.scrollIntoView({ behavior: reducedMotion() ? 'auto' : 'smooth', block: 'start' })
  }
  const download = () => window.location.assign(DOWNLOAD)
  const github = () => window.location.assign(GITHUB)

  // The hero's two actions, again in the close: Download for Mac and the install section (plan, Decisions 2).
  const actions = (where: 'hero' | 'start') => (
    <div ref={where === 'hero' ? heroActions : undefined} className="ms-actions cs-actions">
      <Button variant="primary" size="m" type="button" data-cs-download={where} onClick={download}>{CS.nav.download}</Button>
      <Button variant="secondary" size="m" type="button" data-cs-install-link={where} onClick={go('install')}>{CS.hero.install}</Button>
    </div>
  )

  return (
    <div ref={root} className="ms cs" data-motion={motion || undefined}>
      <header className="ms-bar cs-bar" data-scrolled={scrolled || undefined}>
        <div className="ss-wrap cs-bar-in">
          <a className="ms-brand cs-brand" href="/" aria-label={CS.brand.name}>
            <AppIcon size={32} />
            <span className="ms-brand-word cs-brand-word">{CS.brand.name}</span>
          </a>
          <div className="ms-doors">
            <Button variant="secondary" size="s" type="button" data-cs-github="bar" onClick={github}>{CS.nav.github}</Button>
            <span className="ms-bar-request cs-bar-download" data-shown={barDownload || undefined} inert={!barDownload}><span><Button variant="primary" size="s" type="button" data-cs-download="bar" onClick={download}>{CS.nav.download}</Button></span></span>
          </div>
        </div>
      </header>

      <main>
        {/* The hero: the icon over the headline and line, the two actions and the Open source link, then shot 1 on a band
            the page's width, as Marshal's hero shot sits. */}
        <section id="hero" className="ms-hero" aria-labelledby="cs-hero-title">
          <div className="ss-wrap">
            <div className="ms-hero-copy">
              <AppIcon size={96} className="cs-hero-icon" />
              <h1 id="cs-hero-title" className="ms-hero-title">{CS.hero.title}</h1>
              <p className="ms-lede">{CS.hero.lede}</p>
              {actions('hero')}
              <TextLink className="ms-quiet-action cs-open-link" href={GITHUB} data-cs-open="hero">{CS.hero.openSource}<G.Chevron.Right aria-hidden="true" /></TextLink>
            </div>
          </div>
          <div className="ms-hero-stage">
            <div className="ss-wrap cs-hero-shot"><Shot id={1} label={CS.hero.shot}><HeroShot /></Shot></div>
          </div>
        </section>

        {/* Every session gets a room: the words at the left, shot 2 at the right. */}
        <section id="rooms" className="sf-section cs-section" aria-labelledby="cs-rooms-title">
          <div className="ss-wrap cs-split" data-reveal="">
            <div className="cs-split-words">
              <h2 id="cs-rooms-title" className="ms-h2 cs-h2">{CS.rooms.title}</h2>
              <p className="ss-line">{CS.rooms.line}</p>
            </div>
            <Shot id={2} label={CS.rooms.shot}><RoomShot /></Shot>
          </div>
        </section>

        {/* Bring a teammate in: a centred heading over shot 3, wide. */}
        <section id="teammate" className="sf-section cs-section" aria-labelledby="cs-teammate-title">
          <div className="ss-wrap" data-reveal="">
            <div className="sf-head">
              <h2 id="cs-teammate-title">{CS.teammate.title}</h2>
              <p className="ss-line">{CS.teammate.line}</p>
            </div>
            <div className="cs-wide"><Shot id={3} label={CS.teammate.shot}><JoinShot /></Shot></div>
          </div>
        </section>

        {/* Your team's sessions: shot 4 at the left, the words at the right. A phone leaves the shot off until its crop
            reads (plan, Product shots). */}
        <section id="team" className="sf-section cs-section" aria-labelledby="cs-team-title">
          <div className="ss-wrap cs-split cs-split-flip" data-reveal="">
            <Shot id={4} label={CS.team.shot}><TeamShot /></Shot>
            <div className="cs-split-words">
              <h2 id="cs-team-title" className="ms-h2 cs-h2">{CS.team.title}</h2>
              <p className="ss-line">{CS.team.line}</p>
            </div>
          </div>
        </section>

        {/* Works with: one card of three rows, grouped by how each joins, as msgblast's "Bring your favorites". */}
        <section id="works" className="sf-section cs-section" aria-labelledby="cs-works-title">
          <div className="ss-wrap" data-reveal="">
            <div className="sf-head">
              <h2 id="cs-works-title">{CS.works.title}</h2>
              <p className="ss-line">{CS.works.line}</p>
            </div>
            <ul className="cs-works">
              {CS.works.rows.map((row) => (
                <li key={row.id} className="cs-join" data-cs-join={row.id}>
                  <JoinArt id={row.id} />
                  <div><h3>{row.title}</h3><p>{row.line}</p></div>
                </li>
              ))}
            </ul>
          </div>
        </section>

        {/* Install: cowboy.inc's command row, one per piece, each with Copy. */}
        <section id="install" className="sf-section cs-section" aria-labelledby="cs-install-title">
          <div className="ss-wrap" data-reveal="">
            <div className="sf-head">
              <h2 id="cs-install-title">{CS.install.title}</h2>
              <p className="ss-line">{CS.install.line}</p>
            </div>
            <ol className="cs-install">
              {CS.install.rows.map((row) => <InstallRow key={row.id} row={row} copied={copied === row.id} onCopy={copy(row)} />)}
            </ol>
          </div>
        </section>

        {/* Open source: a band with the words at the left and the repository at the right. */}
        <section id="open" className="cs-section ms-band" aria-labelledby="cs-open-title">
          <div className="ss-wrap cs-open" data-reveal="">
            <div className="cs-open-words">
              <h2 id="cs-open-title" className="ms-h2 cs-h2">{CS.open.title}</h2>
              <p className="ss-line">{CS.open.line}</p>
            </div>
            <div className="cs-repo" data-cs-repo="">
              <p className="cs-repo-name">{CS.open.repository}</p>
              <p className="cs-repo-license">{CS.open.license}</p>
              <Button variant="secondary" size="m" type="button" iconBefore={<G.Outgoing />} data-cs-github="open" onClick={github}>{CS.open.action}</Button>
            </div>
          </div>
        </section>

        {/* The close: the icon again, one line and the hero's two actions, as msgblast closes. */}
        <section id="start" className="cs-section cs-start" aria-labelledby="cs-start-title">
          <div className="ss-wrap cs-start-in" data-reveal="">
            <AppIcon size={72} />
            <h2 id="cs-start-title" className="ms-h2 cs-h2">{CS.start.title}</h2>
            {actions('start')}
          </div>
        </section>
      </main>

      <footer className="ms-foot cs-foot">
        <div className="ss-wrap">
          <div className="ms-foot-grid">
            <div className="ms-foot-brand">
              <a className="ms-brand" href="/" aria-label={CS.brand.name}><AppIcon size={28} /><span className="ms-brand-word">{CS.brand.name}</span></a>
              <small>{CS.footer.copyright}</small>
            </div>
            {CS.footer.groups.map((group) => (
              <nav key={group.title} aria-label={group.title}>
                <h2 className="ms-foot-title">{group.title}</h2>
                {group.links.map(({ label, href }) => <TextLink key={label} href={href} size="s">{label}</TextLink>)}
              </nav>
            ))}
          </div>
          <p className="cs-foot-owners" data-cs-owners="">{CS.footer.owners}</p>
        </div>
      </footer>
    </div>
  )
}
