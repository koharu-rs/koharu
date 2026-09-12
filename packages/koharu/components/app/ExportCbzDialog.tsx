'use client'

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { commands, type ArchiveImageFormat, type ExportConfig } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@koharu/ui/components/dialog'
import { Input } from '@koharu/ui/components/input'

export function ExportCbzDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const [format, setFormat] = useState<ArchiveImageFormat>('png')
  const [config, setConfig] = useState<ExportConfig | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (!open) return
    let active = true
    void call(commands.getExportConfig)
      .then((value) => {
        if (active) setConfig(value)
      })
      .catch(() => undefined)
    return () => {
      active = false
    }
  }, [open])
  const quality = format === 'jpeg' ? config?.jpeg_quality : config?.webp_quality
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className='max-w-sm'>
        <DialogHeader>
          <DialogTitle>{t('export.cbzTitle')}</DialogTitle>
          <DialogDescription>{t('export.entireProject')}</DialogDescription>
        </DialogHeader>
        <label className='grid gap-2 text-xs'>
          {t('export.imageFormat')}
          <select
            aria-label={t('export.imageFormat')}
            className='rounded-md border bg-background p-2'
            value={format}
            onChange={(event) => setFormat(event.target.value as ArchiveImageFormat)}
          >
            <option value='png'>PNG</option>
            <option value='jpeg'>JPEG</option>
            <option value='webp'>WebP</option>
          </select>
        </label>
        {format !== 'png' && config && (
          <label className='grid gap-2 text-xs'>
            {t('export.quality')}
            <Input
              type='number'
              min={1}
              max={100}
              aria-label={t('export.quality')}
              value={quality}
              onChange={(event) => {
                const quality = Math.max(
                  1,
                  Math.min(100, Math.round(Number(event.target.value)) || 1),
                )
                setConfig({
                  ...config,
                  [format === 'jpeg' ? 'jpeg_quality' : 'webp_quality']: quality,
                })
              }}
            />
            <span className='text-muted-foreground'>{t('export.savedQuality')}</span>
          </label>
        )}
        <Button
          disabled={busy || !config}
          onClick={() => {
            if (!config) return
            setBusy(true)
            void call(commands.saveExportConfig, config)
              .then(() => call(commands.exportCbz, format))
              .then(() => onOpenChange(false))
              .catch(() => undefined)
              .finally(() => setBusy(false))
          }}
        >
          {t('export.start')}
        </Button>
      </DialogContent>
    </Dialog>
  )
}
