const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')
const { readFileSync } = require('node:fs')

const { runGitCliff } = require('git-cliff')
const { GitHub, Manifest, registerReleaseType } = require('release-please')
// BaseStrategy is internal: keep the release-please dependency pinned and test this adapter on upgrades.
const { BaseStrategy } = require('release-please/build/src/strategies/base')

function command(program, args) {
  return execFileSync(program, args, { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }).trim()
}

class Koharu extends BaseStrategy {
  async buildReleaseNotes(_commits, _version, tag, latestRelease) {
    assert(latestRelease?.sha, 'The previous release tag must be available')
    const { value: head } = await this.github
      .mergeCommitIterator(this.targetBranch, { maxResults: 1 })
      .next()
    assert.equal(
      command('git', ['rev-parse', 'HEAD']),
      head?.sha,
      'main changed; rerun with a fresh checkout',
    )
    assert.equal(
      command('git', ['status', '--porcelain']),
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
    command('cargo', ['set-version', '--workspace', newVersion.toString()])
    const paths = command('git', ['diff', '--name-only']).split('\n').filter(Boolean)
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
