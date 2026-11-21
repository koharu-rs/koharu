//! What a sequence keeps after its request ends: a new prompt goes to the
//! sequence that holds the longest prefix of it, and a recurrent model's
//! sequence goes back to a checkpoint of its state within that prefix.

use anyhow::{Context, Result};
use koharu_llama::{LlamaStateSeqFlags, SeqState, token::LlamaToken};

use super::{Engine, Job};

/// Only instructions and history are worth sharing; a shorter common prefix is
/// the chat template's header.
const MIN_SHARED: usize = 64;

/// How far short of a prompt's end a recurrent model's checkpoint is taken
/// at least: past the chat template's generation prompt, which the next turn
/// renders otherwise, as Qwen3.5's empty think block that history leaves out.
const CHECKPOINT_BACK: usize = 16;

pub(super) struct Checkpoint {
    at: usize,
    state: SeqState,
}

impl Engine<'_> {
    /// Takes a free sequence for a prompt starting with `head`, holding as much
    /// of it as any sequence does; answers it and how many tokens it holds.
    /// When `logits` is set, the head's last token is left to decode again, as
    /// logits come from it.
    pub(super) fn place(
        &mut self,
        head: &[LlamaToken],
        logits: bool,
        free: &mut Vec<i32>,
    ) -> Result<(i32, usize)> {
        let (best, common) = self
            .held
            .iter()
            .enumerate()
            .map(|(id, held)| (id as i32, common_prefix(held, head)))
            .max_by_key(|&(id, common)| (common, free.contains(&id)))
            .context("the model has no sequences")?;
        let common = if logits {
            common.min(head.len().saturating_sub(1))
        } else {
            common
        };
        let in_place = free.iter().position(|&id| id == best);
        let id = match in_place.filter(|_| common >= MIN_SHARED) {
            Some(at) => free.swap_remove(at),
            None => self.take_emptiest(free)?,
        };
        if common < MIN_SHARED {
            self.forget(id);
            return Ok((id, 0));
        }
        if id != best {
            // Sequences in separate KV streams copy whole streams; the data
            // moves at the next decode, and what follows is taken off now.
            self.context.copy_kv_cache_seq(best, id, None, None)?;
            self.held[id as usize] = self.held[best as usize].clone();
            self.checkpoints[id as usize] = None;
        }
        let kept =
            match self
                .context
                .clear_kv_cache_seq(Some(id as u32), Some(common as u32), None)?
            {
                true => Some(common),
                false => self.rewind(id, common),
            };
        let Some(kept) = kept else {
            self.forget(id);
            return Ok((id, 0));
        };
        self.held[id as usize].truncate(kept);
        Ok((id, kept))
    }

    /// Takes sequence `id` back to its checkpoint, if one lies within the
    /// `common` tokens a new prompt shares: its recurrent state is restored,
    /// and its attention cache cut back to match. Answers the tokens it
    /// then holds.
    fn rewind(&mut self, id: i32, common: usize) -> Option<usize> {
        let checkpoint = self.checkpoints[id as usize]
            .as_ref()
            .filter(|checkpoint| (MIN_SHARED..=common).contains(&checkpoint.at))?;
        let at = checkpoint.at;
        self.context.state_seq_set(&checkpoint.state, id).ok()?;
        self.context
            .clear_kv_cache_seq(Some(id as u32), Some(at as u32), None)
            .ok()
            .filter(|&cut| cut)
            .map(|_| at)
    }

    /// The free sequence that holds the least.
    fn take_emptiest(&self, free: &mut Vec<i32>) -> Result<i32> {
        let at = free
            .iter()
            .enumerate()
            .min_by_key(|&(_, &id)| self.held[id as usize].len())
            .map(|(at, _)| at)
            .context("no sequence is free")?;
        Ok(free.swap_remove(at))
    }

    /// Decodes `head` in sequence `id` from `start` up to a checkpoint short
    /// of its end, which it saves, when the model is recurrent; answers where
    /// decoding goes on, or `None` when the request ended.
    pub(super) fn checkpoint(
        &mut self,
        job: &Job,
        id: i32,
        head: &[LlamaToken],
        start: usize,
    ) -> Result<Option<usize>> {
        // On a micro-batch boundary from position 0, so every token decodes
        // in the micro-batch it would with the whole prompt decoded at once,
        // and the logits, the answer included, come out the same.
        let ubatch = self.context.n_ubatch() as usize;
        let mark = head.len().saturating_sub(CHECKPOINT_BACK) / ubatch * ubatch;
        // Before any image too: a page's turn shows it after the history,
        // which the next turn shares.
        if !(self.recurrent && mark > start && mark >= MIN_SHARED) {
            return Ok(Some(start));
        }
        if !self.feed(job, id, head, start..mark, false)? {
            return Ok(None);
        }
        let state = self
            .context
            .state_seq_get(id, LlamaStateSeqFlags::PARTIAL_ONLY)
            .map_err(|error| anyhow::anyhow!("couldn't save a checkpoint: {error:?}"))?;
        self.checkpoints[id as usize] = Some(Checkpoint { at: mark, state });
        Ok(Some(mark))
    }

    /// Empties sequence `id`.
    pub(super) fn forget(&mut self, id: i32) {
        // A whole sequence's removal always succeeds.
        let _ = self.context.clear_kv_cache_seq(Some(id as u32), None, None);
        self.held[id as usize].clear();
        self.checkpoints[id as usize] = None;
    }
}

fn common_prefix(held: &[LlamaToken], head: &[LlamaToken]) -> usize {
    held.iter()
        .zip(head)
        .take_while(|(held, head)| held == head)
        .count()
}
