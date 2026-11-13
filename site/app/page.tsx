import { CowchatSite } from '../components/cowchat-site/CowchatSite'

// Cowchat's marketing page (site/README.md). The layout sets the title and description from copy.ts and puts Gallop's
// dark class on <html> before the first paint when the system asks for dark, so a dark page does not flash light.
export default function Home() {
  return <CowchatSite />
}
