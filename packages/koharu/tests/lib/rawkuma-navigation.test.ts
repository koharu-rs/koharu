import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { runInNewContext } from 'node:vm'

import { afterEach, describe, expect, it, vi } from 'vitest'

const script = readFileSync(resolve(process.cwd(), '../bridge/src/rawkuma-navigation.js'), 'utf8')
const chapter = 'https://rawkuma.net/manga/example/chapter-38.5/'
const cleanups: Array<() => void> = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
})

function install() {
  document.body.innerHTML = `
    <a aria-label="Prev" href="/manga/example/chapter-38/">Prev</a>
    <a aria-label="Next" href="/manga/example/chapter-39/">Next</a>
    <button aria-label="Home" _="on click go to url \`https://rawkuma.net/manga/example/\`">Home</button>
    <button pop-content="#chapter-lists-head">Chapter 38.5</button>
    <div id="chapter-lists-head" hidden>
      <a href="/manga/example/chapter-39/">Chapter 39</a>
      <a href="${chapter}">Chapter 38.5</a>
      <a href="https://unrelated.test/tracker">Unrelated</a>
    </div>`
  const assign = vi.fn()
  const target = {
    location: { href: chapter, assign },
    addEventListener: (type: string, listener: EventListener, options: boolean) => {
      window.addEventListener(type, listener, options)
      cleanups.push(() => window.removeEventListener(type, listener, options))
    },
  }
  class Observer extends MutationObserver {
    constructor(callback: MutationCallback) {
      super(callback)
      cleanups.push(() => this.disconnect())
    }
  }
  runInNewContext(script, { window: target, document, URL, Element, MutationObserver: Observer })
  return assign
}

describe('Rawkuma chapter navigation', () => {
  it('navigates Prev/Next/Home before page capture handlers consume a chapter click', () => {
    const assign = install()
    const pageCapture = vi.fn()
    window.addEventListener('click', pageCapture, true)
    for (const [label, destination] of [
      ['Prev', '/manga/example/chapter-38/'],
      ['Next', '/manga/example/chapter-39/'],
      ['Home', '/manga/example/'],
    ]) {
      const event = new MouseEvent('click', { bubbles: true, cancelable: true })
      document.querySelector(`[aria-label="${label}"]`)!.dispatchEvent(event)
      expect(event.defaultPrevented).toBe(true)
      expect(assign).toHaveBeenLastCalledWith('https://rawkuma.net' + destination)
    }
    expect(pageCapture).not.toHaveBeenCalled()
    window.removeEventListener('click', pageCapture, true)
  })
  it('replaces the Tippy control with an exact-URL select that preserves fractional chapters', () => {
    const assign = install()
    const select = document.querySelector<HTMLSelectElement>('select[data-koharu-chapters]')!
    expect(select.options).toHaveLength(2)
    expect(select.value).toBe(chapter)
    expect(document.querySelector<HTMLElement>('[pop-content]')!.hidden).toBe(true)
    select.value = 'https://rawkuma.net/manga/example/chapter-39/'
    select.dispatchEvent(new Event('change', { bubbles: true, cancelable: true }))
    expect(assign).toHaveBeenLastCalledWith(select.value)
  })
})
