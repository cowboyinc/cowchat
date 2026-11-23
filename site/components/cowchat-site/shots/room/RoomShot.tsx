'use client'
import { APP, SAMPLE } from '../../copy'
import { AppWindow, CLAUDE_CODE_MARK, Fold, Message, Pane, PaneHead, PullRequestRow } from '../parts'

/* Shot 2, a room per session (plan, Product shots; REHEARSAL.md beside this file). A still of one session's room: the
   agent messages it folded away, then its report with the pull request row and two screenshots under it, at the
   app's thumbnail height of 120 with corners of 12 (`AttachmentThumbnail`). On a phone the header and the fold are
   cropped off, and the second screenshot wraps under the first and runs off the bottom. */

export function RoomShot() {
  const r = SAMPLE.room
  return (
    <AppWindow className="cs-room-app">
      <Pane>
        <PaneHead title={r.session} line={APP.connected} />
        <div className="cs-app-feed">
          <Fold label={APP.agentMessages(2)} />
          <Message author={r.session} mark={CLAUDE_CODE_MARK} time={r.time} text={r.report}>
            <PullRequestRow pr={r.pr} />
            <div className="cs-app-thumbs">
              {r.shots.map((src) => <img key={src} className="cs-app-thumb" src={src} alt="" />)}
            </div>
          </Message>
        </div>
      </Pane>
    </AppWindow>
  )
}
