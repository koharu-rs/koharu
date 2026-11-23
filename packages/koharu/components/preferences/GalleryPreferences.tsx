'use client'

import { useMutation, useQuery } from '@tanstack/react-query'
import { CircleCheck, LoaderCircle, LogIn, LogOut } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { queryClient, refresh } from '@/lib/queries'
import { commands, type Site } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'

/** The gallery's sites, the ones the built-in extensions read, each with who is signed in to it
 * and its sign-in, which opens the site's own page in a window of its own: what the user signs in
 * to there, the gallery reads with. */
export function GalleryPreferences() {
  const { t } = useTranslation()
  const sites = useQuery({ queryKey: ['sites'], queryFn: () => commands.listSites() }).data ?? []
  return (
    <PreferencePage
      title={t('settings.gallery.title')}
      description={t('settings.gallery.description')}
    >
      <PreferenceSection title={t('settings.gallery.accounts')}>
        {sites.map((site) => (
          <SiteRow key={site.id} site={site} />
        ))}
      </PreferenceSection>
    </PreferencePage>
  )
}

function SiteRow({ site }: { site: Site }) {
  const { t } = useTranslation()
  const accountKey = ['account', site.id]
  const account = useQuery({
    queryKey: accountKey,
    queryFn: () => commands.getAccount(site.id),
    enabled: site.signIn,
  })
  // The sign-in answers once its window closes, when who is signed in may have changed.
  const session = useMutation({
    mutationFn: (signIn: boolean) =>
      signIn ? commands.signInSite(site.id) : commands.signOutSite(site.id),
    onSettled: async () => {
      // Discard cached library pages so another account never sees this one's books.
      for (const queryKey of [
        ['site', site.id],
        ['site-all', 'site', site.id],
      ]) {
        await queryClient.cancelQueries({ queryKey })
        queryClient.removeQueries({ queryKey })
      }
      await refresh(accountKey)
    },
  })
  return (
    <PreferenceRow title={site.name}>
      <div className='flex min-w-0 items-center justify-end gap-3'>
        {!site.signIn ? (
          <span className='text-[11px] text-muted-foreground'>
            {t('settings.gallery.noSignIn')}
          </span>
        ) : account.isPending ? (
          <LoaderCircle className='size-3.5 animate-spin text-muted-foreground' />
        ) : (
          <>
            {account.data && (
              <span className='flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground'>
                <CircleCheck className='size-3.5 shrink-0 text-primary' />
                <span className='truncate' title={account.data}>
                  {t('settings.gallery.signedIn', { name: account.data })}
                </span>
              </span>
            )}
            <Button
              size='sm'
              variant={account.data ? 'destructive' : 'outline'}
              className='h-8 shrink-0 gap-1.5 text-[11px]'
              disabled={session.isPending}
              aria-busy={session.isPending}
              aria-label={
                account.data ? t('settings.gallery.signOutLabel', { site: site.name }) : undefined
              }
              onClick={() => session.mutate(!account.data)}
            >
              {account.data ? <LogOut className='size-3.5' /> : <LogIn className='size-3.5' />}
              {t(account.data ? 'settings.gallery.signOut' : 'settings.gallery.signIn')}
            </Button>
          </>
        )}
      </div>
    </PreferenceRow>
  )
}
