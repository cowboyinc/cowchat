'use client'
import { Spinner } from '../../gallop'
import * as G from '../../icons'
import { APP, SAMPLE } from '../../copy'
import { AppWindow, Initial, Pane, PaneHead } from '../parts'

/* Shot 4, team sessions (plan, Product shots; REHEARSAL.md beside this file). A still of the Team page (TeamView.swift):
   the title and branch count with the team switch and the search, the column labels, then two people with their
   branches, each branch's repository, pull request with passing checks, and state, two of them Working with the
   turning mark, which is Gallop's spinner in Icon/Tertiary as SessionWorkingMark draws it. Pull request and State
   start on the left under their labels, and the labels sit 8 over the first person. On a phone the switch, the search
   and the Repo column are left out, since a Mac window has no phone width (REHEARSAL.md). */

export function TeamShot() {
  const t = SAMPLE.team
  const count = t.people.reduce((total, person) => total + person.rows.length, 0)
  const end = (
    <>
      <span className="cs-app-team-switch"><Initial name={t.name} /><span>{t.name}</span><G.Chevron.Small.Down /></span>
      <span className="cs-app-search"><G.Search /><span>{APP.searchTeam}</span></span>
    </>
  )
  return (
    <AppWindow className="cs-team-app">
      <Pane>
        <PaneHead title={APP.team} line={APP.branches(count)} end={end} />
        <div className="cs-app-team">
          <div className="cs-app-team-row cs-app-team-labels">
            <span>{APP.columns.branch}</span><span className="cs-app-team-repo">{APP.columns.repo}</span><span>{APP.columns.pr}</span><span>{APP.columns.state}</span>
          </div>
          {t.people.map((person) => (
            <div key={person.name} className="cs-app-team-group">
              <div className="cs-app-team-person"><Initial name={person.name} /><span>{person.name}</span></div>
              {person.rows.map((row) => (
                <div key={row.branch} className="cs-app-team-row">
                  <span className="cs-app-team-branch">{row.branch}</span>
                  <span className="cs-app-team-repo">{row.repo}</span>
                  <span className="cs-app-team-pr">{row.pr && <>{`#${row.pr}`}<span className="cs-app-team-mark cs-app-pr-checks"><G.Success /></span></>}</span>
                  <span className="cs-app-team-state" data-working={row.state === 'Working' || undefined}>{row.state === 'Working' ? APP.working : APP.idle}<span className="cs-app-team-mark">{row.state === 'Working' && <Spinner className="cs-app-working" />}</span></span>
                </div>
              ))}
            </div>
          ))}
        </div>
      </Pane>
    </AppWindow>
  )
}
