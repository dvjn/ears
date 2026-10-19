// Live transcription over a rolling buffer.
//
// whisper.cpp is not natively streaming, so this uses the LocalAgreement-2
// policy from whisper_streaming: re-transcribe a rolling audio buffer with
// word timestamps and only commit the words that two consecutive hypotheses
// agree on. Committed audio is trimmed from the buffer so it isn't decoded
// again.

use crate::audio::SAMPLE_RATE;
use crate::whisper::{Model, Options};
use anyhow::Result;
use std::sync::Arc;

/// Decode once this much new audio has arrived.
pub const MIN_CHUNK: usize = SAMPLE_RATE as usize;
/// Hard cap on un-trimmed audio.
const MAX_BUFFER: usize = SAMPLE_RATE as usize * 30;
/// Segments the model itself flags as likely non-speech are dropped.
const NO_SPEECH_MAX: f32 = 0.6;

#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Commits the longest common prefix of two consecutive hypotheses.
#[derive(Default)]
pub struct HypothesisBuffer {
    committed: Vec<Word>,
    buffer: Vec<Word>, // previous hypothesis tail
    new: Vec<Word>,    // latest hypothesis
    pub last_committed_time: f64,
}

impl HypothesisBuffer {
    pub fn insert(&mut self, words: Vec<Word>, offset: f64) {
        self.new = words
            .into_iter()
            .map(|w| Word {
                start: w.start + offset,
                end: w.end + offset,
                text: w.text,
            })
            .filter(|w| w.start > self.last_committed_time - 0.1)
            .collect();

        if let (Some(first), false) = (self.new.first(), self.committed.is_empty()) {
            if (first.start - self.last_committed_time).abs() < 1.0 {
                // Drop words repeating the tail of what's already committed
                // (n-gram overlap, n up to 5).
                let max = self.committed.len().min(self.new.len()).min(5);
                for n in 1..=max {
                    let tail = &self.committed[self.committed.len() - n..];
                    let head = &self.new[..n];
                    if tail.iter().zip(head).all(|(a, b)| same_word(a, b)) {
                        self.new.drain(..n);
                        break;
                    }
                }
            }
        }
    }

    /// Words newly agreed on by the last two hypotheses.
    pub fn flush(&mut self) -> Vec<Word> {
        let agreed = self
            .new
            .iter()
            .zip(&self.buffer)
            .take_while(|(a, b)| same_word(a, b))
            .count();
        let commit: Vec<Word> = self.new.drain(..agreed).collect();
        if let Some(last) = commit.last() {
            self.last_committed_time = last.end;
        }
        self.buffer = std::mem::take(&mut self.new);
        self.committed.extend(commit.iter().cloned());
        commit
    }

    pub fn pending(&self) -> String {
        join(&self.buffer)
    }
}

/// Hypotheses often differ only in punctuation or case ("so" vs "so,");
/// those still count as agreeing.
fn same_word(a: &Word, b: &Word) -> bool {
    let norm = |w: &Word| -> String {
        w.text
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    norm(a) == norm(b)
}

fn join(words: &[Word]) -> String {
    words.iter().map(|w| w.text.as_str()).collect()
}

pub struct OnlineProcessor {
    model: Arc<Model>,
    language: Option<String>,
    audio: Vec<f32>,
    offset: f64, // seconds of audio already trimmed away
    hyp: HypothesisBuffer,
}

impl OnlineProcessor {
    pub fn new(model: Arc<Model>, language: Option<String>) -> Self {
        Self {
            model,
            language,
            audio: Vec::new(),
            offset: 0.0,
            hyp: HypothesisBuffer::default(),
        }
    }

    pub fn insert_audio(&mut self, pcm: &[f32]) {
        self.audio.extend_from_slice(pcm);
    }

    fn transcribe(&self) -> Result<Vec<Word>> {
        let segments = self.model.transcribe(
            &self.audio,
            &Options {
                language: self.language.as_deref(),
                prompt: None,
                temperature: 0.0,
                word_timestamps: true,
            },
        )?;
        Ok(segments
            .into_iter()
            .filter(|s| s.no_speech_prob <= NO_SPEECH_MAX)
            .flat_map(|s| s.words)
            .map(|w| Word {
                start: w.start,
                end: w.end,
                text: w.text,
            })
            .collect())
    }

    /// Decode the buffer; returns (newly committed text, pending text).
    pub fn process(&mut self) -> Result<(String, String)> {
        let words = self.transcribe()?;
        self.hyp.insert(words, self.offset);
        let commit = self.hyp.flush();
        if !commit.is_empty() {
            self.trim(self.hyp.last_committed_time);
        }
        if self.audio.len() > MAX_BUFFER {
            // Nothing is committing (e.g. long silence); don't grow forever.
            let excess = (self.audio.len() - MAX_BUFFER) as f64 / SAMPLE_RATE as f64;
            self.trim(self.offset + excess);
        }
        Ok((join(&commit), self.hyp.pending()))
    }

    /// Commit everything that is left.
    pub fn finish(&mut self) -> Result<String> {
        let (commit, _) = self.process()?;
        Ok(commit + &self.hyp.pending())
    }

    fn trim(&mut self, t: f64) {
        let cut = ((t - self.offset) * SAMPLE_RATE as f64) as usize;
        if cut > 0 {
            self.audio.drain(..cut.min(self.audio.len()));
            self.offset = t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(start: f64, end: f64, text: &str) -> Word {
        Word {
            start,
            end,
            text: text.into(),
        }
    }

    #[test]
    fn commits_agreed_prefix() {
        let mut hyp = HypothesisBuffer::default();
        hyp.insert(vec![w(0.0, 0.5, " hello"), w(0.5, 1.0, " word")], 0.0);
        assert!(hyp.flush().is_empty());
        hyp.insert(
            vec![
                w(0.0, 0.5, " hello"),
                w(0.5, 1.0, " world"),
                w(1.0, 1.5, " again"),
            ],
            0.0,
        );
        assert_eq!(hyp.flush(), vec![w(0.0, 0.5, " hello")]);
        assert_eq!(hyp.last_committed_time, 0.5);
        assert_eq!(hyp.pending(), " world again");
    }

    #[test]
    fn drops_repeated_committed_tail() {
        let mut hyp = HypothesisBuffer::default();
        hyp.insert(vec![w(0.0, 0.5, " one")], 0.0);
        hyp.flush();
        hyp.insert(vec![w(0.0, 0.5, " one")], 0.0);
        hyp.flush();
        // After trimming, the re-decoded buffer repeats " one" at its start.
        hyp.insert(vec![w(0.0, 0.4, " one"), w(0.6, 1.0, " two")], 0.5);
        assert_eq!(
            hyp.new.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            [" two"]
        );
    }

    #[test]
    fn punctuation_and_case_do_not_block_agreement() {
        let mut hyp = HypothesisBuffer::default();
        hyp.insert(vec![w(0.0, 0.4, " And"), w(0.4, 0.8, " so")], 0.0);
        hyp.flush();
        hyp.insert(
            vec![w(0.0, 0.4, " and"), w(0.4, 0.8, " so,"), w(0.8, 1.2, " my")],
            0.0,
        );
        assert_eq!(hyp.flush().len(), 2);
        // A re-decoded tail with different punctuation is still a repeat.
        hyp.insert(vec![w(0.0, 0.4, " So,"), w(0.5, 0.9, " my")], 0.75);
        assert_eq!(
            hyp.new.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            [" my"]
        );
    }
}
