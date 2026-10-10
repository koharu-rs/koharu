import { defaultTiers, defineConfig, tierPresets } from 'sponsorkit'

const specialTier = {
  title: 'Special Sponsors',
  // Non-cash support sorts between backers (0) and past sponsors (-1).
  monthlyDollars: -0.5,
  preset: tierPresets.large,
}

/** @type {import('sponsorkit').Provider} */
const specialSponsors = {
  name: 'special-sponsors',
  async fetchSponsors() {
    return [
      { login: 'getsentry', name: 'Sentry', websiteUrl: 'https://sentry.io/' },
      { login: 'mintlify', name: 'Mintlify', websiteUrl: 'https://www.mintlify.com/' },
      { login: 'openai', name: 'OpenAI', websiteUrl: 'https://openai.com/' },
    ].map((sponsor) => ({
      sponsor: {
        ...sponsor,
        type: 'Organization',
        avatarUrl: `https://github.com/${sponsor.login}.png`,
      },
      monthlyDollars: specialTier.monthlyDollars,
    }))
  },
}

export default defineConfig({
  github: {
    login: 'mayocream',
    type: 'user',
  },
  outputDir: '.',
  formats: ['svg'],
  providers: ['github', 'patreon', specialSponsors],
  width: 800,
  includePastSponsors: true,
  tiers: [...defaultTiers, specialTier],
  onSvgGenerated(svg) {
    return svg.replace(/[\t ]+$/gm, '')
  },
  onSponsorsAllFetched(sponsors) {
    let anonymousCount = 0

    return sponsors.map((sponsorship) => {
      if (sponsorship.sponsor.name || sponsorship.sponsor.login) return sponsorship

      anonymousCount += 1

      return {
        ...sponsorship,
        sponsor: {
          ...sponsorship.sponsor,
          login: `anonymous-${anonymousCount}`,
          name: 'Anonymous',
          avatarUrl: '',
          websiteUrl: undefined,
          linkUrl: undefined,
        },
      }
    })
  },
})
