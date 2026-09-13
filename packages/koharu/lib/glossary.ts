import type { GlossaryCandidate } from '@koharu/bridge/protocol'

export function glossaryTranslationBatches(candidates: GlossaryCandidate[]) {
  const batches: { index: number; source: string }[][] = []
  let batch: { index: number; source: string }[] = []
  let bytes = 0
  const encoder = new TextEncoder()
  candidates.forEach((candidate, index) => {
    if (candidate.suggested_target.trim() || !candidate.source.trim()) return
    const size = encoder.encode(candidate.source).length
    if (batch.length && (batch.length === 24 || bytes + size > 4096)) {
      batches.push(batch)
      batch = []
      bytes = 0
    }
    batch.push({ index, source: candidate.source })
    bytes += size
  })
  if (batch.length) batches.push(batch)
  return batches
}
