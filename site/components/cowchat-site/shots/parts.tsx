'use client'
import type { ReactNode } from 'react'
import * as G from '../icons'
import './shots.css'

/* The Mac app's parts as the four shots draw them (plan, Product shots; site/README.md, The
   page). Each is a recreation of a view in the Mac prototype as P9 builds it (cowchat-mac-proto,
   native/CowchatMac/Sources/CowchatMac/), drawn from Gallop's tokens and icons at the app's sizes, and compared against
   P9's renders. None is a control: each shot's root is inert. */

// The window behind the panes, cropped at the sidebar's edge, as Marshal's hero is cropped at the dashboard's sidebar.
export function AppWindow({ children, className = '' }: { children: ReactNode; className?: string }) {
  return <div className={`cs-app ${className}`.trim()} inert>{children}</div>
}

// A page's pane: ContentView's rounded detail column, Surface/600 on the window with Border/Default.
export function Pane({ children, className = '' }: { children: ReactNode; className?: string }) {
  return <div className={`cs-app-pane ${className}`.trim()}>{children}</div>
}

// A room's or page's header: its title in the display face and the caption under it (ContentView's chatHeader and
// PageParts.swift's PageHeader). A room's caption takes the accent while an agent in it is working.
export function PaneHead({ title, line, active, end }: { title: string; line?: string; active?: boolean; end?: ReactNode }) {
  return (
    <div className="cs-app-head">
      <div className="cs-app-head-words">
        <div className="cs-app-title">{title}</div>
        {line && <div className="cs-app-line" data-active={active || undefined}>{line}</div>}
      </div>
      {end && <div className="cs-app-head-end">{end}</div>}
    </div>
  )
}

// A Claude Code session's mark, Anthropic's Claude icon, unaltered (site/README.md, The marks). Every
// session in the shots runs Claude Code, so a session draws this one mark in a room and in the Join sheet alike.
export const CLAUDE_CODE_MARK = '/assets/cowchat-site/claude-icon.svg'

// A person's or agent's initial in its disc, as a message row and the Team page draw it.
export function Initial({ name }: { name: string }) {
  return <span className="cs-app-initial" aria-hidden="true">{name.charAt(0).toUpperCase()}</span>
}

// A message's words, with a leading mention drawn in the accent (ContentView, mentionMarks).
function Words({ text }: { text: string }) {
  const match = /^(@[\w-]+)(\s.*)$/.exec(text)
  if (!match) return <>{text}</>
  return <><span className="cs-app-mention">{match[1]}</span>{match[2]}</>
}

// One message: the author's disc, the author and time, then the words and what the message carries. A session's disc
// draws its agent's mark, as AgentAvatar draws the agent app's icon when it can resolve one, inset 6% of its size.
export function Message({ author, mark, time, text, children }: { author: string; mark?: string; time: string; text: string; children?: ReactNode }) {
  return (
    <div className="cs-app-message">
      {mark ? <span className="cs-app-initial cs-app-avatar" aria-hidden="true"><img src={mark} alt="" /></span> : <Initial name={author} />}
      <div className="cs-app-message-main">
        <div className="cs-app-message-head"><span className="cs-app-author">{author}</span><span className="cs-app-time">{time}</span></div>
        <div className="cs-app-words"><Words text={text} /></div>
        {children}
      </div>
    </div>
  )
}

// GitHub's pull request mark on a 16 grid, as `PullRequestMark` in MessageAttachmentViews.swift draws it: Gallop has no
// pull request icon, so the app draws its own, and this is the same path.
function PullRequestMark() {
  return (
    <svg className="cs-app-pr-mark" viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="4" cy="3.5" r="1.75" /><circle cx="4" cy="12.5" r="1.75" /><circle cx="12" cy="12.5" r="1.75" />
      <path d="M4 5.25V10.75M12 10.75V6.5Q12 4 9.5 4H7.5M9.25 2.25L7.5 4L9.25 5.75" />
    </svg>
  )
}

export type PullRequest = { repo: string; number: number; title: string; adds: number; dels: number }

// The pull request row a message carries (`PullRequestRow`): the mark, the repository and number, the title, then the
// passing checks and the line counts. On a phone it takes the app's two-line form, the one its ViewThatFits picks when
// one line does not fit: the title wraps beside the name and the status goes under it, 24 in.
export function PullRequestRow({ pr }: { pr: PullRequest }) {
  return (
    <div className="cs-app-pr">
      <span className="cs-app-pr-main">
        <PullRequestMark /><span className="cs-app-pr-name">{`${pr.repo} #${pr.number}`}</span>
        <span className="cs-app-pr-title">{pr.title}</span>
      </span>
      <span className="cs-app-pr-status">
        <span className="cs-app-pr-checks"><G.Success /></span>
        <span className="cs-app-pr-adds">{`+${pr.adds}`}</span>
        <span className="cs-app-pr-dels">{`−${pr.dels}`}</span>
      </span>
    </div>
  )
}

// An agent's thinking pill (`ThinkingPulsePill` in RoomConnectState.swift). The app hangs it under the agent's latest
// message; the hero hangs it under the question it answers (hero/REHEARSAL.md).
export function ThinkingPill({ label }: { label: string }) {
  return <div className="cs-app-pill"><span className="cs-app-pill-dot" aria-hidden="true" /><span>{label}</span></div>
}

// A run of agent messages folded for people (MessagesForPeople.swift), drawn closed.
export function Fold({ label }: { label: string }) {
  return <div className="cs-app-fold"><G.Chatbot /><span>{label}</span><G.Chevron.Small.Right /></div>
}

// Opens and closes its content in place, the way the room's feed moves when a message arrives or leaves: the height
// opens first and the content fades in after it, and on the way out the content fades before the height closes.
export function Arrive({ open, className = '', children }: { open: boolean; className?: string; children: ReactNode }) {
  return <div className={`cs-app-arrive ${className}`.trim()} data-open={open || undefined}><div>{children}</div></div>
}
