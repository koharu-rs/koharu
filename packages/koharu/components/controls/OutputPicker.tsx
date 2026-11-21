'use client'

import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { MenuPanel } from '@/components/controls/MenuPanel'
import { PickerSearch } from '@/components/controls/PickerSearch'
import { languageNames } from '@/lib/translation'
import { Combobox, ComboboxEmpty, ComboboxItem, ComboboxList } from '@koharu/ui/components/combobox'
import { ScrollArea } from '@koharu/ui/components/scroll-area'

/** The translator's target language, picked from the list as the model is beside it, and saved as
 * soon as it's picked. */
export function OutputPicker({
  targetLanguage,
  disabled = false,
  saving = false,
  onBack,
  onChange,
}: {
  targetLanguage: string
  disabled?: boolean
  saving?: boolean
  onBack: () => void
  onChange: (targetLanguage: string) => void
}) {
  const { t, i18n } = useTranslation()
  const [query, setQuery] = useState('')
  const names = useMemo(() => languageNames(i18n.language), [i18n.language])

  return (
    <MenuPanel title={t('outputPicker.title')} backDisabled={saving} onBack={onBack}>
      <Combobox<string>
        inline
        open
        items={Object.keys(names)}
        value={targetLanguage}
        inputValue={query}
        readOnly={disabled || saving}
        itemToStringLabel={(tag) => names[tag] ?? tag}
        onInputValueChange={setQuery}
        onValueChange={(tag) => tag && tag !== targetLanguage && onChange(tag)}
      >
        <div className='mb-0.5 border-b border-border/60 pb-1'>
          <PickerSearch
            query={query}
            label={t('outputPicker.search')}
            onClear={() => setQuery('')}
          />
        </div>
        {/* Its children come only while nothing matches, so it's empty, and hidden, otherwise. */}
        <ComboboxEmpty className='flex px-2.5 py-5 empty:hidden'>
          {t('outputPicker.noResults')}
        </ComboboxEmpty>
        <ScrollArea className='max-h-72 min-w-0' viewportClassName='max-h-72 overscroll-contain'>
          <ComboboxList
            aria-label={t('outputPicker.targetLanguage')}
            className='max-h-none overflow-visible p-0 pt-0.5'
          >
            {(tag: string) => (
              <ComboboxItem key={tag} value={tag}>
                {names[tag]}
              </ComboboxItem>
            )}
          </ComboboxList>
        </ScrollArea>
      </Combobox>
    </MenuPanel>
  )
}
