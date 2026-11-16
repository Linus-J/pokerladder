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
    if (to === location.pathname + location.search) return
    if (replace) history.replaceState(null, '', to)
    else history.pushState(null, '', to)
    this.sync()
    scrollTo(0, 0)
  }
}

export const router = new Router()

/** Click handler for in-app links: `<a href="/x" onclick={link}>`. */
export function link(e: MouseEvent) {
  if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return
  const a = e.currentTarget as HTMLAnchorElement
  if (a.origin !== location.origin) return
  e.preventDefault()
  router.go(a.pathname + a.search)
}
