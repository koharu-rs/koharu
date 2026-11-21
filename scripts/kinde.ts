#!/usr/bin/env bun
// SPDX-License-Identifier: GPL-3.0-only
// Adapted from Alexia/kandle-downloader (Azxiana):
// https://github.com/Alexia/kandle-downloader/blob/71f7e5346eddcc031a6899c091f13a2d80ba73fd/kandle-downloader.js
// License: https://www.gnu.org/licenses/gpl-3.0.html
// bun scripts/kinde.ts ASIN [ASIN ...] [-o data/kindle]

import { createDecipheriv, pbkdf2Sync } from 'node:crypto'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { parseArgs } from 'node:util'

import { Archive } from 'bun'
import { chromium, type Page } from 'playwright'

type Book = {
  asin: string
  contentGuid: string
  karamelToken: { token: string; expiresAt: number }
}

type Render = {
  locations?: number[]
  pages: { children: { imageReference?: string; elementId: number }[] }[]
  manifest: {
    cdn: { baseUrl: string; authParameter: string | null; isEncrypted: boolean }
    cdnResources: { url: string; authParameter: string | null }[]
  }
}

async function render(
  page: Page,
  book: Book,
  position: number,
  locationMap = false,
): Promise<Render> {
  const query = new URLSearchParams({
    version: '3.0',
    asin: book.asin,
    contentType: 'FullBook',
    revision: book.contentGuid,
    fontFamily: 'Bookerly',
    fontSize: '4.95',
    lineHeight: '1.4',
    dpi: '160',
    height: '808',
    width: '2560',
    marginBottom: '0',
    marginLeft: '9',
    marginRight: '9',
    marginTop: '0',
    maxNumberColumns: '1',
    theme: 'dark',
    packageType: 'TAR',
    encryptionVersion: 'NONE',
    numPage: '1',
    skipPageCount: '0',
    startingPosition: String(position),
    bundleImages: 'false',
  })
  if (locationMap) query.set('locationMap', 'true')
  const response = await page.request.get(`https://read.amazon.co.jp/renderer/render?${query}`, {
    headers: { Accept: 'application/x-tar', 'x-amz-rendering-token': book.karamelToken.token },
    maxRedirects: 0,
    timeout: 30000,
  })
  if (!response.ok()) throw new Error(`Kindle render: HTTP ${response.status()}`)
  const files = await new Archive(await response.body()).files()
  await response.dispose()
  const pages: Render['pages'] = []
  for (const [name, file] of files) {
    if (name.startsWith('page_data_')) pages.push(...JSON.parse(await file.text()))
  }
  return {
    manifest: JSON.parse(await files.get('manifest.json')!.text()),
    locations: locationMap
      ? JSON.parse(await files.get('location_map.json')!.text()).locations
      : undefined,
    pages,
  }
}

function decrypt(bytes: Buffer, book: Book): Buffer {
  const { token, expiresAt } = book.karamelToken
  const secret = token.slice(expiresAt % 60, (expiresAt % 60) + 40)
  const encoded = bytes.toString('utf8')
  const salt = Buffer.from(encoded.slice(0, 24), 'base64')
  const iv = Buffer.from(encoded.slice(24, 48), 'base64')
  const ciphertext = Buffer.from(encoded.slice(48), 'base64')
  const key = pbkdf2Sync(secret, salt, 1000, 16, 'sha256')
  const decipher = createDecipheriv('aes-128-gcm', key, iv)
  decipher.setAAD(Buffer.from(secret.slice(0, 9)))
  decipher.setAuthTag(ciphertext.subarray(-16))
  return Buffer.concat([decipher.update(ciphertext.subarray(0, -16)), decipher.final()])
}

function imageExtension(bytes: Buffer): string {
  if (bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return 'png'
  if (bytes.subarray(0, 3).equals(Buffer.from([255, 216, 255]))) return 'jpg'
  if (bytes.toString('ascii', 0, 4) === 'RIFF' && bytes.toString('ascii', 8, 12) === 'WEBP')
    return 'webp'
  if (/^GIF8[79]a$/.test(bytes.toString('ascii', 0, 6))) return 'gif'
  throw new Error('Unknown Kindle image format')
}

const { values, positionals: asins } = parseArgs({
  allowPositionals: true,
  options: {
    cdp: { type: 'string', default: 'chrome' },
    o: { type: 'string', short: 'o', default: 'data/kindle' },
  },
})
if (!asins.length || asins.some((asin) => !/^[A-Z0-9]{10}$/.test(asin)))
  throw new Error('Provide a Kindle ASIN')

console.log('Connecting to Chrome; approve the remote debugging dialog if prompted.')
await using browser = await chromium.connectOverCDP(values.cdp, {
  noDefaults: true,
  timeout: 60000,
})
const context = browser.contexts()[0]

for (const asin of asins) {
  await using page = await context.newPage()
  await page.goto(`https://read.amazon.co.jp/manga/${asin}`, { waitUntil: 'domcontentloaded' })
  if (
    new URL(page.url()).pathname.startsWith('/ap/') ||
    new URL(page.url()).pathname === '/landing'
  ) {
    throw new Error('Sign in to Kindle JP in Chrome first')
  }
  const book = JSON.parse((await page.locator('#bookInfo').textContent({ timeout: 5000 }))!) as Book
  const initial = await render(page, book, 0, true)
  const directory = path.resolve(values.o, asin)
  await mkdir(directory, { recursive: true })
  const seen = new Set<number>()

  // Locations overlap and spreads contain multiple images. Keep first occurrences in source order.
  for (const position of new Set(initial.locations!)) {
    const result = position === 0 ? initial : await render(page, book, position)
    const { cdn, cdnResources } = result.manifest
    for (const image of result.pages.flatMap((page) => page.children)) {
      if (!image.imageReference || seen.has(image.elementId)) continue
      const resource = cdnResources.find((resource) => resource.url === image.imageReference)!
      // Per-resource signatures cover the exact URL; only encrypted CDN URLs accept the reader token.
      const url = new URL(
        `${cdn.baseUrl.replace(/\/$/, '')}/${resource.url}?${cdn.authParameter ?? resource.authParameter}`,
      )
      if (cdn.isEncrypted)
        url.search += `&token=${encodeURIComponent(book.karamelToken.token)}&expiration=${book.karamelToken.expiresAt}`
      if (url.protocol !== 'https:' || !url.hostname.endsWith('.cloudfront.net'))
        throw new Error('Unexpected Kindle image host')
      const response = await fetch(url, { redirect: 'error', signal: AbortSignal.timeout(30000) })
      if (!response.ok) throw new Error(`Kindle image: HTTP ${response.status}`)
      let bytes: Buffer = Buffer.from(await response.arrayBuffer())
      if (cdn.isEncrypted) bytes = decrypt(bytes, book)
      const file = `page_${String(seen.size + 1).padStart(4, '0')}.${imageExtension(bytes)}`
      await writeFile(path.join(directory, file), bytes)
      seen.add(image.elementId)
      console.log(`${asin}: ${file}`)
    }
  }

  console.log(`Saved ${directory}`)
}
