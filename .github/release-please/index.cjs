const assert = require('node:assert/strict')
const { execFile } = require('node:child_process')
const { readFileSync } = require('node:fs')
const { promisify } = require('node:util')

const { runGitCliff } = require('git-cliff')
const { GitHub, Manifest, registerReleaseType } = require('release-please')
// BaseStrategy is internal: keep the release-please dependency pinned and test this adapter on upgrades.
const { BaseStrategy } = require('release-please/build/src/strategies/base')

// Asynchronous, so Node's fetch closes idle GitHub connections while cargo runs: a
// blocked event loop leaves one GitHub has already closed, and the next request fails on it.
async function command(program, args) {
  const options = { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }
  const { stdout } = await promisify(execFile)(program, args, options)
  return stdout.trim()
}

class Koharu extends BaseStrategy {
  async buildReleaseNotes(_commits, _version, tag, latestRelease) {
    assert(latestRelease?.sha, 'The previous release tag must be available')
    const { value: head } = await this.github
      .mergeCommitIterator(this.targetBranch, { maxResults: 1 })
      .next()
    assert.equal(
      await command('git', ['rev-parse', 'HEAD']),
      head?.sha,
      'main changed; rerun with a fresh checkout',
    )
    assert.equal(
      await command('git', ['status', '--porcelain']),
      '',
      'Release preparation requires a clean checkout',
    )

    // The virtual tag renders the next release without creating a tag before review.
    const { stdout } = await runGitCliff(
      [
        '--unreleased',
        '--tag',
        tag.toString(),
        '--strip',
        'all',
        `${latestRelease.sha}..HEAD`,
      ],
      { stdio: 'pipe' },
    )
    return stdout.trim()
  }

  async buildUpdates({ newVersion, changelogEntry }) {
    await command('cargo', ['set-version', '--workspace', newVersion.toString()])
    const paths = (await command('git', ['diff', '--name-only'])).split('\n').filter(Boolean)
    assert(
      paths.includes('Cargo.toml') && paths.includes('Cargo.lock'),
      'Cargo must update the workspace version and lockfile',
    )
    assert(
      paths.every((path) => /(^|\/)Cargo\.toml$/.test(path) || path === 'Cargo.lock'),
      'Unexpected cargo-edit output',
    )
    const updates = paths.map((path) => {
      const content = readFileSync(path, 'utf8')
      return { path, createIfMissing: false, updater: { updateContent: () => content } }
    })
    const changelog = `${changelogEntry}\n\n${readFileSync('CHANGELOG.md', 'utf8')}`
    updates.push({
      path: 'CHANGELOG.md',
      createIfMissing: false,
      updater: { updateContent: () => changelog },
    })
    return updates
  }

  // release-please finds the version in the release PR body's heading, and CHANGELOG.md
  // keeps it; the release page already titles the notes with the version.
  async buildReleases(mergedPullRequest, options) {
    const releases = await super.buildReleases(mergedPullRequest, options)
    return releases.map((release) => ({
      ...release,
      notes: release.notes.replace(/^#{2,} \[?\d+\.\d+\.\d+.*\s*/, ''),
    }))
  }
}

// A custom release type avoids rewriting Cargo's inherited workspace versions in JavaScript.
registerReleaseType('koharu', (options) => new Koharu(options))

module.exports = async function releasePlease({ owner, repo, token }) {
  const github = await GitHub.create({ owner, repo, token })
  const manifest = await Manifest.fromManifest(
    github,
    'main',
    '.github/release-please/config.json',
    '.github/release-please/manifest.json',
  )
  const releases = (await manifest.createReleases()).filter(Boolean)
  assert(releases.length <= 1, 'Koharu has a single workspace release')
  if (releases.length) return releases[0].tagName
  await manifest.createPullRequests()
  return ''
}
