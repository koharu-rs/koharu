import type { Metadata } from 'next'
import { ThemeProvider } from 'next-themes'
import localFont from 'next/font/local'

import './globals.css'
import Providers from '@/app/providers'

const notoSans = localFont({
  src: './fonts/NotoSans-VF.ttf',
  weight: '100 900',
  variable: '--font-noto-sans',
})

const notoSansCJK = localFont({
  src: './fonts/NotoSansCJK-VF.otf',
  weight: '100 900',
  variable: '--font-noto-sans-cjk',
})

export const metadata: Metadata = {
  title: 'Koharu',
}

function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode
}>) {
  return (
    <html
      lang='en-US'
      className={`${notoSans.variable} ${notoSansCJK.variable}`}
      suppressHydrationWarning
      style={{ backgroundColor: 'transparent' }}
    >
      <body className='antialiased' style={{ backgroundColor: 'transparent' }}>
        <ThemeProvider attribute='class' defaultTheme='system' enableSystem>
          <Providers>{children}</Providers>
        </ThemeProvider>
      </body>
    </html>
  )
}

export default RootLayout
