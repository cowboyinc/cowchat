/* The site follows the system's light or dark scheme. Gallop's dark values live in its unlayered `.dark` block, which
   its @theme aliases read at :root, so the class goes on <html>. The layout runs THEME_SCRIPT before the first paint so
   a dark page does not flash light, and the home page keeps the class in step when the system's scheme changes, as the
   prototype's theme module does (without its ?feature.<surface>.theme override, which belongs to its guard). No React
   here: the layout, a server component, reads the script. */

const DARK_QUERY = '(prefers-color-scheme: dark)'

export const THEME_SCRIPT = `if (window.matchMedia("${DARK_QUERY}").matches) document.documentElement.classList.add("dark");`

export const systemDark = () => { try { return window.matchMedia(DARK_QUERY).matches } catch { return false } }

// Puts the class on <html> to match the system's scheme, and returns whether it is dark.
export function applyTheme() {
  const dark = systemDark()
  document.documentElement.classList.toggle('dark', dark)
  return dark
}

// Calls the listener whenever the system's scheme changes, and returns the function that stops it.
export function subscribeSystem(listener: () => void) {
  try {
    const query = window.matchMedia(DARK_QUERY)
    query.addEventListener('change', listener)
    return () => query.removeEventListener('change', listener)
  } catch { return () => {} }
}
