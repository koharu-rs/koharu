// Shared by the macOS CEF source view and the iOS WKWebView. This runs only in
// the external Rawkuma frame and never exposes native commands or credentials.
;(() => {
  if (window.__koharuChapterNavigation) return
  const allowed = (value) => {
    try {
      const url = new URL(value, window.location.href)
      return url.protocol === 'https:' &&
        ['rawkuma.net', 'www.rawkuma.net'].includes(url.hostname) &&
        !url.port &&
        !url.username &&
        !url.password
        ? url.href
        : null
    } catch {
      return null
    }
  }
  if (!allowed(window.location.href)) return
  window.__koharuChapterNavigation = true
  const target = (event) =>
    event.target instanceof Element ? event.target : event.target?.parentElement
  const selectFor = (event) => target(event)?.closest('select[data-koharu-chapters]')
  // A native select must receive its default action without page-level popunder
  // handlers consuming the gesture or replacing the document underneath it.
  for (const name of ['pointerdown', 'mousedown', 'touchstart']) {
    window.addEventListener(
      name,
      (event) => {
        if (selectFor(event)) event.stopImmediatePropagation()
      },
      true,
    )
  }
  window.addEventListener(
    'click',
    (event) => {
      if (selectFor(event)) {
        event.stopImmediatePropagation()
        return
      }
      const control = target(event)?.closest(
        'a[aria-label="Next"], a[aria-label="Prev"], button[aria-label="Home"], [id^="chapter-lists-"] a[href]',
      )
      if (!control) return
      const source =
        control.getAttribute('href') ??
        control.getAttribute('_')?.match(/go\s+to\s+url\s+`([^`]+)`/)?.[1]
      const url = source && allowed(source)
      if (!url) return
      event.preventDefault()
      event.stopImmediatePropagation()
      window.location.assign(url)
    },
    true,
  )
  window.addEventListener(
    'change',
    (event) => {
      const select = selectFor(event)
      const url = select && allowed(select.value)
      if (!url) return
      event.preventDefault()
      event.stopImmediatePropagation()
      window.location.assign(url)
    },
    true,
  )
  const enhance = () => {
    for (const button of document.querySelectorAll('button[pop-content^="#chapter-lists-"]')) {
      if (button.dataset.koharuEnhanced) continue
      const panel = document.getElementById(button.getAttribute('pop-content').slice(1))
      const links = [...(panel?.querySelectorAll('a[href]') ?? [])]
        .map((link) => ({
          label: link.textContent.trim(),
          url: allowed(link.getAttribute('href')),
        }))
        .filter((item) => item.url && item.label)
      if (!links.length) continue
      const select = document.createElement('select')
      select.dataset.koharuChapters = 'true'
      select.setAttribute('aria-label', button.textContent.trim() || document.title)
      select.className = button.className
      select.style.cssText =
        'color-scheme:dark;min-width:180px;max-width:100%;cursor:pointer;appearance:auto'
      for (const link of links) {
        const option = document.createElement('option')
        option.value = link.url
        option.textContent = link.label
        option.selected = link.url === window.location.href
        select.append(option)
      }
      button.dataset.koharuEnhanced = 'true'
      button.hidden = true
      button.style.display = 'none'
      button.after(select)
    }
  }
  const start = () => {
    enhance()
    new MutationObserver(enhance).observe(document.documentElement, {
      childList: true,
      subtree: true,
    })
  }
  if (document.documentElement) start()
  else document.addEventListener('DOMContentLoaded', start, { once: true })
})()
