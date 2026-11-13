import { CS } from '../../components/cowchat-site/copy'

// The site is one page now, so /how-it-works sends the reader to it. The static export ignores Next's redirects, so the
// page does it with a meta refresh, which React places in the head, and a plain link for a reader whose browser does not
// follow one. The link's words are the brand's name from copy.ts, so the page adds no copy of its own.
export const metadata = { title: CS.meta.title, robots: { index: false } }

export default function HowItWorks() {
  return (
    <main className="mx-auto w-full max-w-3xl px-6 py-20">
      <meta httpEquiv="refresh" content="0; url=/" />
      <a className="type-body-m text-text-secondary underline underline-offset-2 hover:text-text-primary" href="/">{CS.brand.name}</a>
    </main>
  )
}
