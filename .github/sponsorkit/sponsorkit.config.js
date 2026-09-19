import { defineConfig } from 'sponsorkit'

export default defineConfig({
  github: {
    login: 'mayocream',
    type: 'user',
  },
  outputDir: '.',
  formats: ['svg'],
  providers: ['github', 'patreon'],
  width: 800,
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
          avatarUrl: undefined,
          websiteUrl: undefined,
          linkUrl: undefined,
        },
      }
    })
  },
})
