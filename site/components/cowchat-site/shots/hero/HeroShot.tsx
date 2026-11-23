'use client'
import { useEffect, useRef, useState } from 'react'
import { APP, SAMPLE } from '../../copy'
import { AppWindow, Arrive, CLAUDE_CODE_MARK, Message, Pane, PaneHead, PullRequestRow, ThinkingPill } from '../parts'

/* Shot 1, the hero (plan, Product shots; REHEARSAL.md beside this file). A session's room with a teammate in it: the
   teammate mentions the session, it thinks, then answers with its pull request's new line counts. It opens on its
   poster, the payoff with all three messages, so the server's render, reduced motion and a screenshot all show the
   story. While it is in view and motion is allowed it loops: the two newer messages leave, the question arrives, the
   session thinks under the question while the header's caption names it active, the answer arrives and holds. The
   app hangs the pill under the session's own last message; here it follows the question, so the order reads as the
   story (REHEARSAL.md). On a phone the feed sits at its end, as the app scrolls a feed to its newest message, and the
   header is cropped off. */

type Step = 'poster' | 'quiet' | 'ask' | 'thinking'

// The beats, in ms from the poster: leave, ask, think, answer, then hold the answer until the loop starts again.
const BEATS: [number, Step][] = [[2600, 'quiet'], [4000, 'ask'], [5300, 'thinking'], [7600, 'poster']]
const LOOP = 10400

export function HeroShot() {
  const root = useRef<HTMLDivElement>(null)
  const [step, setStep] = useState<Step>('poster')
  // Set while a reset puts the poster back from another step, so it lands at once (shots.css, data-jump).
  const [jump, setJump] = useState(false)
  const h = SAMPLE.hero
  const teammate = SAMPLE.people.teammate

  useEffect(() => {
    const node = root.current
    if (!node) return
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)')
    let timers: number[] = []
    let visible = false
    const stop = () => { timers.forEach((id) => window.clearTimeout(id)); timers = [] }
    const run = () => {
      stop()
      setJump(true)
      setStep('poster')
      if (reduce.matches || !visible || document.hidden) return
      for (const [at, next] of BEATS) timers.push(window.setTimeout(() => setStep(next), at))
      timers.push(window.setTimeout(run, LOOP))
    }
    const seen = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; run() }, { threshold: 0.4 })
    seen.observe(node)
    reduce.addEventListener('change', run)
    document.addEventListener('visibilitychange', run)
    return () => { stop(); seen.disconnect(); reduce.removeEventListener('change', run); document.removeEventListener('visibilitychange', run) }
  }, [])

  // The jump ends a frame after the poster is drawn, so the next beat moves again.
  useEffect(() => {
    if (!jump) return
    const frame = window.requestAnimationFrame(() => setJump(false))
    return () => window.cancelAnimationFrame(frame)
  }, [jump])

  const asked = step !== 'quiet'
  const answered = step === 'poster'
  return (
    <div ref={root} className="cs-hero-app" data-cs-hero-step={step} data-jump={jump || undefined}>
      <AppWindow>
        <Pane>
          <PaneHead title={h.session} line={step === 'thinking' ? APP.active(h.session) : APP.connected} active={step === 'thinking'} />
          <div className="cs-app-feed">
            <Message author={h.session} mark={CLAUDE_CODE_MARK} time={h.times.report} text={h.report}>
              <PullRequestRow pr={h.pr} />
            </Message>
            <Arrive open={asked}>
              <Message author={`${teammate.name} · @${teammate.handle}`} time={h.times.ask} text={h.ask} />
            </Arrive>
            <Arrive open={step === 'thinking'} className="cs-app-pulse"><ThinkingPill label={APP.thinking(h.session)} /></Arrive>
            <Arrive open={answered}>
              <Message author={h.session} mark={CLAUDE_CODE_MARK} time={h.times.reply} text={h.reply}>
                <PullRequestRow pr={{ ...h.pr, adds: h.replyAdds, dels: h.replyDels }} />
              </Message>
            </Arrive>
          </div>
        </Pane>
      </AppWindow>
    </div>
  )
}
