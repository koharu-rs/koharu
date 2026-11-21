'use client'

import { openUrl } from '@tauri-apps/plugin-opener'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

import { cn } from '@koharu/ui/lib/utils'

/** Markdown, as release notes and the agent's replies come, in the editor's small type; a web
 * link opens in the browser. */
export function Markdown({ children, className }: { children: string; className?: string }) {
  return (
    <div
      className={cn(
        'min-w-0 text-xs leading-5 break-words [&_a]:font-medium [&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2 [&_blockquote]:my-2 [&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_code]:rounded [&_code]:bg-muted [&_code]:px-1 [&_code]:py-0.5 [&_code]:break-all [&_h1]:mt-4 [&_h1]:mb-2 [&_h1]:text-sm [&_h1]:font-semibold [&_h1]:text-foreground [&_h2]:mt-4 [&_h2]:mb-2 [&_h2]:text-sm [&_h2]:font-semibold [&_h2]:text-foreground [&_h3]:mt-3 [&_h3]:mb-1 [&_h3]:font-semibold [&_h3]:text-foreground [&_hr]:my-3 [&_hr]:border-border [&_img]:h-auto [&_img]:max-w-full [&_li]:my-0.5 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:my-2 [&_pre]:my-2 [&_pre]:max-w-full [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-muted [&_pre]:p-2 [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:break-normal [&_table]:my-2 [&_table]:w-full [&_table]:table-fixed [&_td]:border [&_td]:border-border [&_td]:p-1.5 [&_th]:border [&_th]:border-border [&_th]:p-1.5 [&_th]:text-left [&_th]:font-medium [&_th]:text-foreground [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5 [&>*:first-child]:mt-0 [&>*:last-child]:mb-0',
        className,
      )}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a({ href, children }) {
            return (
              <a
                href={href}
                onClick={(event) => {
                  if (!href || !/^https?:\/\//i.test(href)) return
                  event.preventDefault()
                  void openUrl(href).catch(() => undefined)
                }}
              >
                {children}
              </a>
            )
          },
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  )
}
