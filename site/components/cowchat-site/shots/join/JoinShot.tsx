'use client'
import { Button } from '../../gallop'
import * as G from '../../icons'
import { APP, SAMPLE } from '../../copy'
import { AppWindow, CLAUDE_CODE_MARK, Pane, PaneHead } from '../parts'

/* Shot 3, bring someone in (plan, Product shots; REHEARSAL.md beside this file). A still of the Join sheet
   (JoinRoomSheet.swift) over the Inbox it was opened from, dimmed as the window dims under a sheet. The sheet names the
   teammate's room and who invited the reader, then lists No session and two of the reader's live sessions, each with
   its agent's mark and repository; the first session is picked to bring. The Inbox's first row is the invitation.
   A Claude Code session's mark is Anthropic's Claude icon, unaltered, as the app draws it. On a phone the sheet stands
   alone over the dimmed Inbox's top. */

export function JoinShot() {
  const j = SAMPLE.join
  const teammate = SAMPLE.people.teammate
  const rows = [{ title: teammate.name, preview: APP.invitation, room: j.room, time: j.time, unread: true }, ...j.inbox.map((row) => ({ ...row, unread: false }))]
  return (
    <AppWindow className="cs-join-app">
      <Pane>
        <PaneHead title={APP.inbox} end={<Button variant="secondary" size="s" type="button">{APP.markAllRead}</Button>} />
        <div className="cs-app-inbox">
          {rows.map((row) => (
            <div key={row.title} className="cs-app-inbox-row" data-unread={row.unread || undefined}>
              <span className="cs-app-unread" aria-hidden="true" />
              <div className="cs-app-inbox-words"><div className="cs-app-inbox-title">{row.title}</div><div className="cs-app-inbox-preview">{row.preview}</div></div>
              <span className="cs-app-inbox-room">{row.room}</span>
              <span className="cs-app-inbox-time">{row.time}</span>
            </div>
          ))}
        </div>
      </Pane>
      <div className="cs-app-scrim" />
      <div className="cs-app-sheet">
        <div className="cs-app-sheet-title">{j.room}</div>
        <div className="cs-app-sheet-line">{APP.invitedBy(teammate.name)}</div>
        <div className="cs-app-sheet-label">{APP.addSession}</div>
        <div className="cs-app-choices">
          <div className="cs-app-choice"><span className="cs-app-choice-mark" /><span className="cs-app-choice-name">{APP.noSession}</span></div>
          {j.sessions.map((session, i) => (
            <div key={session.name} className="cs-app-choice" data-picked={i === 0 || undefined}>
              <img className="cs-app-choice-mark" src={CLAUDE_CODE_MARK} alt="" />
              <span className="cs-app-choice-name">{session.name}</span>
              <span className="cs-app-choice-repo">{session.repo}</span>
              <span className="cs-app-choice-tick">{i === 0 && <G.Checkmark />}</span>
            </div>
          ))}
        </div>
        <div className="cs-app-sheet-actions">
          <Button variant="secondary" size="s" type="button">{APP.decline}</Button>
          <span className="cs-app-sheet-end">
            <Button variant="secondary" size="s" type="button">{APP.cancel}</Button>
            <Button variant="primary" size="s" type="button">{APP.join}</Button>
          </span>
        </div>
      </div>
    </AppWindow>
  )
}
