// Copy for Cowchat's marketing page at cowchat.cowboy.inc (site/README.md). It was designed in the prototype
// (cowboyinc/prototype-site, /cowchat-site, PR 531), where Codex wrote every heading, line and alt text in one pass over
// the built page's screenshots, 3 Nov 2026; the hero is Patrick's pick and the action labels carry the plan's names. The
// layout reads them and adds no words of its own. The commands, links and agent names are facts, not copy. The server's
// command is Install in this repository's README, and the Mac app's installs the cask in cowboyinc/homebrew-tap. The
// Claude Code plugin's is plugins/cowchat-claude/README.md and the Codex hooks' is plugins/cowchat-codex/README.md. The
// footer's Cowboy links, which point at the prototype's own pages there, point at the live sites here.
//
// Codex wrote three strings on 12 November 2026 for this port, after a review found the install section offered the
// plugin and the hooks without the Mac app: the install section's line, the Codex hooks row's line and the label of the
// README links, Setup. Both READMEs say to connect an agent in the Mac app first, and both hooks fail without it. Codex
// kept the plugin row's line. Each README link goes to that plugin's README on GitHub, which has the full setup.

export const GITHUB = 'https://github.com/cowboyinc/cowchat'
export const RELEASES = `${GITHUB}/releases`
// The latest release's page until PR 53 gives the DMG a stable address.
export const DOWNLOAD = `${RELEASES}/latest`
export const SKILL = 'https://cowchat.cowboy.inc/skills.txt'

export type CsLink = { label: string; href: string }
export type CsGroup = { title: string; links: CsLink[] }
// One way an agent or a person joins Cowchat, a row of the Works with card.
export type CsJoin = { id: 'agents' | 'shell' | 'people'; title: string; line: string }
// One piece to install, a row of the install section, with the command its Copy puts on the clipboard. `guide` is the
// address of the piece's README, where it has one.
export type CsInstall = { id: 'app' | 'server' | 'plugin' | 'codex'; title: string; line: string; command: string; guide?: string }
// The two agents shown by name and mark. `src` is the owner's own file, unaltered, with its source in site/README.md
// (The marks). `srcDark` is the owner's file for the dark theme, where it publishes one;
// without it `src` draws in both.
export type CsAgent = { id: 'claude-code' | 'codex'; name: string; mark: string; src: string; srcDark?: string }

export const AGENTS: CsAgent[] = [
  { id: 'claude-code', name: 'Claude Code', mark: 'Claude Code', src: '/assets/cowchat-site/claude-spark.svg' },
  { id: 'codex', name: 'Codex', mark: 'Codex', src: '/assets/cowchat-site/openai-blossom-black.svg', srcDark: '/assets/cowchat-site/openai-blossom-white.svg' },
]

export const CS = {
  meta: {
    title: 'Cowchat',
    description: 'Keep the conversation with your teammate and the agents doing the work.',
  },
  brand: { name: 'Cowchat' },
  nav: {
    github: 'GitHub',
    download: 'Download for Mac',
  },
  // Patrick's pick from the third hero round, 3 Nov 2026. The headline stays as it is.
  hero: {
    title: 'Group chat for coding agents',
    lede: 'Keep the conversation with your teammate and the agents doing the work.',
    install: 'Install',
    openSource: 'Open source',
    shot: 'A teammate requests a cart change, and the session replies with its updated pull request.',
  },
  rooms: {
    title: 'A room per session',
    line: 'Talk to each session in the room where it shares its results.',
    shot: 'A search results report has a pull request and two shop screenshots attached.',
  },
  teammate: {
    title: 'Bring someone in',
    line: 'Work through a problem with a teammate and the sessions you’re both running.',
    shot: 'A Join sheet over the Inbox shows two available sessions, with the first selected.',
  },
  team: {
    title: 'Team sessions',
    line: 'Follow the team’s live sessions and see where each one stands.',
    shot: 'The Team page lists branches under two people, with two sessions marked Working.',
  },
  works: {
    title: 'Works with your tools',
    line: 'Bring the tools you use and the people you build with.',
    rows: [
      { id: 'agents', title: 'Claude Code and Codex', line: 'Sessions join their rooms automatically through the plugin and hooks.' },
      { id: 'shell', title: 'Any agent with a shell', line: 'Use the CLI and skill file to tell it how to join a room.' },
      { id: 'people', title: 'People', line: 'Sign in with Google or GitHub, then add a friend by handle.' },
    ] as CsJoin[],
  },
  install: {
    title: 'Install Cowchat',
    line: 'Connect an agent in the Mac app before using the Claude Code plugin or Codex hooks.',
    copy: 'Copy',
    copied: 'Copied',
    guide: 'Setup',
    rows: [
      { id: 'app', title: 'Mac app', line: 'First launch installs the Claude Code plugin and guides you through trusting the Codex hook.', command: 'brew install --cask cowboyinc/tap/cowchat' },
      { id: 'server', title: 'Server and CLI', line: 'Run locally on Mac or Linux.', command: 'brew install cowboyinc/tap/cowchat' },
      { id: 'plugin', title: 'Claude Code plugin', line: 'Connect Claude Code sessions to their rooms.', command: 'claude plugin install cowchat --marketplace cowboyinc/cowchat', guide: `${GITHUB}/blob/main/plugins/cowchat-claude/README.md` },
      { id: 'codex', title: 'Codex hooks', line: 'Connect Codex sessions to their rooms.', command: 'cowchat-codex install', guide: `${GITHUB}/blob/main/plugins/cowchat-codex/README.md` },
    ] as CsInstall[],
  },
  open: {
    title: 'Open source',
    line: 'Run the server on your own machine and connect through the local CLI without an account.',
    repository: 'cowboyinc/cowchat',
    license: 'MIT or Apache-2.0',
    action: 'View on GitHub',
  },
  start: {
    title: 'A shared home for sessions',
  },
  footer: {
    groups: [
      { title: 'Cowchat', links: [{ label: 'GitHub', href: GITHUB }, { label: 'Releases', href: RELEASES }, { label: 'Skill file', href: SKILL }] },
      { title: 'Cowboy', links: [{ label: 'cowboy.inc', href: 'https://cowboy.inc' }, { label: 'General Store', href: 'https://cloud.cowboy.inc/general-store' }, { label: 'Marshal', href: 'https://marshal.cowboy.inc' }] },
      { title: 'Legal', links: [{ label: 'Terms', href: 'https://cowboy.inc/terms' }, { label: 'Privacy', href: 'https://cowboy.inc/privacy' }] },
    ] as CsGroup[],
    owners: 'Agent names and marks belong to their respective owners.',
    copyright: '© 2026 Cowboy Labs, Inc.',
  },
}

// The shots' invented sample, written by Codex on 3 Nov 2026 and pasted as written, except that
// the hero's question reads "now", since it arrives in view. No real person, company or repository appears.
export const SAMPLE = {
  people: {
    you: { name: 'Nella Verran', handle: 'nella' },
    teammate: { name: 'Ivo Selden', handle: 'ivo' },
    other: { name: 'Tessa Orven', handle: 'tessa' },
  },
  hero: {
    session: 'cart-total',
    report: 'Cart totals now include delivery before checkout. Checked discount rounding and the checkout tests.',
    pr: { repo: 'shelf', number: 184, title: 'Include delivery in cart totals', adds: 64, dels: 12 },
    ask: '@cart-total Can you keep delivery visible when a discount is applied?',
    reply: 'Delivery stays visible after applying a discount. Added a regression test; checkout tests pass.',
    replyAdds: 73,
    replyDels: 14,
    times: { report: '18 min ago', ask: 'now', reply: 'now' },
  },
  room: {
    session: 'search-grid',
    report: 'Aligned prices in the widget search results. Checked the eight-card grid and the linked Widget page, including its $249.00 price and Add to cart button.',
    pr: { repo: 'shelf', number: 187, title: 'Align prices in search results', adds: 42, dels: 19 },
    time: '8 min ago',
    // The prototype's own synthetic shop pages (its public/assets/chat/browser/), drawn as the report's two screenshots.
    shots: ['/assets/cowchat-site/shop-home.jpg', '/assets/cowchat-site/shop-widget.jpg'],
  },
  join: {
    room: 'search-grid',
    sessions: [{ name: 'cart-total', repo: 'shelf' }, { name: 'stock-sync', repo: 'parcel' }],
    inbox: [
      { title: 'stock-sync', preview: 'Stock updates now retry after a timeout. Tests pass.', room: 'stock-sync', time: '6 min ago' },
      { title: 'Tessa Orven', preview: 'I checked the receipt link. It opens the right order.', room: 'receipt-link', time: '12 min ago' },
    ],
    time: '2 min ago',
  },
  team: {
    name: 'Mosswick',
    people: [
      { name: 'Nella Verran', rows: [
        { branch: 'cart-total', repo: 'shelf', pr: 184, state: 'Idle' },
        { branch: 'stock-sync', repo: 'parcel', pr: null, state: 'Working' },
        { branch: 'receipt-link', repo: 'parcel', pr: 181, state: 'Idle' },
      ] },
      { name: 'Ivo Selden', rows: [
        { branch: 'search-grid', repo: 'shelf', pr: 187, state: 'Idle' },
        { branch: 'filter-reset', repo: 'shelf', pr: 189, state: 'Working' },
        { branch: 'parcel-retry', repo: 'parcel', pr: 176, state: 'Idle' },
      ] },
    ] as { name: string; rows: { branch: string; repo: string; pr: number | null; state: 'Working' | 'Idle' }[] }[],
  },
}

// The Mac app's own words in the shots, as the prototype draws them (cowchat-mac-proto, native/CowchatMac/Sources/
// CowchatMac/): its placeholders, marked `// copy:` there, so they change when the app's copy pass changes them.
export const APP = {
  connected: '1 agent connected', // RoomSidebarPresentation.swift, the room header's presence line
  active: (name: string) => `${name} active`, // RoomSidebarPresentation.swift, the same line while an agent works
  thinking: (name: string) => `${name} is thinking…`, // ThinkingPulsePresentation.swift, label(for:)
  agentMessages: (count: number) => (count === 1 ? '1 agent message' : `${count} agent messages`), // MessagesForPeople.swift
  inbox: 'Inbox', // InboxView.swift
  markAllRead: 'Mark all read', // InboxView.swift
  invitation: 'Invited you to join.', // InboxStore.swift, invitationLine
  invitedBy: (name: string) => `Invited by ${name}`, // JoinRoomSheet.swift
  addSession: 'Add a session', // JoinRoomSheet.swift
  noSession: 'No session', // JoinRoomSheet.swift
  decline: 'Decline', // JoinRoomSheet.swift
  cancel: 'Cancel', // JoinRoomSheet.swift
  join: 'Join', // JoinRoomSheet.swift
  team: 'Team', // TeamView.swift
  branches: (count: number) => `${count} branches`, // TeamView.swift
  searchTeam: 'Search people or branches', // TeamView.swift
  columns: { branch: 'Branch', repo: 'Repo', pr: 'Pull request', state: 'State' }, // TeamView.swift
  working: 'Working', // TeamStore.swift
  idle: 'Idle', // TeamStore.swift
}
