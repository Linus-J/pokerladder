// A minimal history-API router: the app has a handful of flat pages.

class Router {
  path = $state(location.pathname)
  query = $state(new URLSearchParams(location.search))

  constructor() {
    addEventListener('popstate', () => this.sync())
  }

  private sync() {
    this.path = location.pathname
    this.query = new URLSearchParams(location.search)
  }

  go(to: string, replace = false) {
    const hash = to.includes('#') ? to.slice(to.indexOf('#')) : ''
    if (to !== location.pathname + location.search + location.hash) {
      if (replace) history.replaceState(null, '', to)
      else history.pushState(null, '', to)
      this.sync()
    }
    if (hash) scrollToHash(hash)
    else scrollTo(0, 0)
  }
}

/** Scrolls to `#id` once the page has rendered it (pages can render a moment late). */
export function scrollToHash(hash: string, tries = 20) {
  const el = document.getElementById(decodeURIComponent(hash.slice(1)))
  if (el) el.scrollIntoView()
  else if (tries > 0) setTimeout(() => scrollToHash(hash, tries - 1), 50)
}

export const router = new Router()

/** Click handler for in-app links: `<a href="/x" onclick={link}>`. */
export function link(e: MouseEvent) {
  if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return
  const a = e.currentTarget as HTMLAnchorElement
  if (a.origin !== location.origin) return
  e.preventDefault()
  router.go(a.pathname + a.search + a.hash)
}
