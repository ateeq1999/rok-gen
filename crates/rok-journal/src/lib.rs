//! The edit journal: how rok-gen undoes its own changes without ever
//! writing marker comments into user code.
//!
//! After a transaction is committed, every changed file is diffed (Rust by
//! tokens, anything else by lines) and each changed region is stored as a
//! [`Run`]: the inserted units, the text they replaced, and a few units of
//! context on either side. Reverting finds each run again in the *current*
//! file by its units, which ignores formatting, so a run is still found
//! after rustfmt, after lines are added above it, or after its function
//! moves. A run that can't be found, or matches ambiguously, is *drifted*:
//! it's reported and nothing is modified.
//!
//! Created files are recorded by content hash and removed on revert only
//! if they're unchanged.

mod units;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use similar::{Algorithm, DiffTag};

pub use units::Granularity;
use units::{Unit, units};

/// The journal file, relative to the project root.
pub const JOURNAL_FILE: &str = ".rok/journal.json";

/// Context units kept on each side of a run.
const CONTEXT: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("could not read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid journal {}: {message}", path.display())]
    Parse { path: PathBuf, message: String },
    #[error("could not write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not tokenize {file}: {message}")]
    Tokenize { file: String, message: String },
}

/// Every recorded transaction, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub transactions: Vec<TxRecord>,
}

/// One committed transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxRecord {
    pub id: String,
    /// The command that made it, e.g. `make:feature` or `integrate`.
    pub command: String,
    /// What `unlink` matches on, e.g. `comments into posts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub undone: bool,
    pub files: Vec<FileRecord>,
}

/// What a transaction did to one file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileRecord {
    Created {
        file: String,
        hash: String,
    },
    Changed {
        file: String,
        granularity: Granularity,
        runs: Vec<Run>,
    },
}

impl FileRecord {
    #[must_use]
    pub fn file(&self) -> &str {
        match self {
            Self::Created { file, .. } | Self::Changed { file, .. } => file,
        }
    }
}

/// One changed region.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Units just before the region (nearest last).
    pub before: Vec<String>,
    /// Units just after the region.
    pub after: Vec<String>,
    /// The units the transaction put there.
    pub inserted: Vec<String>,
    /// The original text the transaction replaced (empty for an insertion).
    pub removed_text: String,
    /// Whitespace that preceded `removed_text`, restored with it when the
    /// whole region was a deletion.
    #[serde(default)]
    pub removed_lead: String,
}

/// One file's change, as a transaction made it (`\n`-normalized text).
#[derive(Clone, Debug)]
pub struct Change {
    pub file: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// What reverting a file needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Revert {
    Write(String),
    Delete,
}

impl Journal {
    /// Loads `root/.rok/journal.json`, or an empty journal.
    ///
    /// # Errors
    ///
    /// [`JournalError::Read`] or [`JournalError::Parse`].
    pub fn load(root: &Path) -> Result<Self, JournalError> {
        let path = root.join(JOURNAL_FILE);
        if !path.is_file() {
            return Ok(Self {
                version: 1,
                transactions: Vec::new(),
            });
        }
        let text = std::fs::read_to_string(&path).map_err(|source| JournalError::Read {
            path: path.clone(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|err| JournalError::Parse {
            path,
            message: err.to_string(),
        })
    }

    /// Writes the journal back.
    ///
    /// # Errors
    ///
    /// [`JournalError::Write`].
    pub fn save(&self, root: &Path) -> Result<(), JournalError> {
        let path = root.join(JOURNAL_FILE);
        let write = || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
            text.push('\n');
            std::fs::write(&path, text)
        };
        write().map_err(|source| JournalError::Write {
            path: path.clone(),
            source,
        })
    }

    /// Records a committed transaction and returns its id.
    ///
    /// # Errors
    ///
    /// [`JournalError::Tokenize`] when a changed Rust file can't be lexed.
    pub fn record(
        &mut self,
        command: &str,
        label: Option<String>,
        changes: &[Change],
        now_secs: u64,
    ) -> Result<String, JournalError> {
        let mut files = Vec::new();
        for change in changes {
            match (&change.before, &change.after) {
                (None, Some(after)) => files.push(FileRecord::Created {
                    file: change.file.clone(),
                    hash: hash(after),
                }),
                (Some(before), Some(after)) if before != after => {
                    let granularity = Granularity::for_file(&change.file);
                    files.push(FileRecord::Changed {
                        file: change.file.clone(),
                        granularity,
                        runs: diff_runs(&change.file, before, after, granularity)?,
                    });
                }
                _ => {}
            }
        }
        let base = format!("{now_secs}-{}", command.replace(':', "-"));
        let mut id = base.clone();
        let mut counter = 1;
        while self.transactions.iter().any(|tx| tx.id == id) {
            counter += 1;
            id = format!("{base}-{counter}");
        }
        self.transactions.push(TxRecord {
            id: id.clone(),
            command: command.to_string(),
            label,
            undone: false,
            files,
        });
        Ok(id)
    }

    /// The most recent transaction that hasn't been undone.
    #[must_use]
    pub fn last_active(&self) -> Option<&TxRecord> {
        self.transactions.iter().rev().find(|tx| !tx.undone)
    }

    #[must_use]
    pub fn find(&self, id: &str) -> Option<&TxRecord> {
        self.transactions.iter().find(|tx| tx.id == id)
    }

    pub fn mark_undone(&mut self, id: &str) {
        if let Some(tx) = self.transactions.iter_mut().find(|tx| tx.id == id) {
            tx.undone = true;
        }
    }
}

/// How to revert `tx` given the files' current contents (`read` returns
/// `None` for a missing file). Either every file reverts cleanly, or the
/// drifted regions are returned and nothing should be changed.
///
/// # Errors
///
/// One message per drifted region or file.
pub fn plan_revert(
    tx: &TxRecord,
    read: impl Fn(&str) -> Option<String>,
) -> Result<Vec<(String, Revert)>, Vec<String>> {
    let mut actions = Vec::new();
    let mut drift = Vec::new();
    for record in &tx.files {
        match record {
            FileRecord::Created {
                file,
                hash: recorded,
            } => match read(file) {
                Some(current) if hash(&current) == *recorded => {
                    actions.push((file.clone(), Revert::Delete));
                }
                Some(_) => drift.push(format!(
                    "{file} was edited after it was created; delete it by hand if intended"
                )),
                None => {}
            },
            FileRecord::Changed {
                file,
                granularity,
                runs,
            } => {
                let Some(current) = read(file) else {
                    drift.push(format!("{file} no longer exists"));
                    continue;
                };
                match revert_runs(file, &current, runs, *granularity) {
                    Ok(text) => actions.push((file.clone(), Revert::Write(text))),
                    Err(reasons) => drift.extend(reasons),
                }
            }
        }
    }
    if drift.is_empty() {
        Ok(actions)
    } else {
        Err(drift)
    }
}

/// The runs that turn `before` into `after`.
///
/// # Errors
///
/// [`JournalError::Tokenize`].
pub fn diff_runs(
    file: &str,
    before: &str,
    after: &str,
    granularity: Granularity,
) -> Result<Vec<Run>, JournalError> {
    let tokenize = |text: &str| {
        units(text, granularity).map_err(|message| JournalError::Tokenize {
            file: file.to_string(),
            message,
        })
    };
    let old = tokenize(before)?;
    let new = tokenize(after)?;
    let old_keys: Vec<&str> = old.iter().map(|unit| unit.text.as_str()).collect();
    let new_keys: Vec<&str> = new.iter().map(|unit| unit.text.as_str()).collect();

    // Merge adjacent non-equal ops into one region each.
    let mut regions: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> = Vec::new();
    for op in similar::capture_diff_slices(Algorithm::Myers, &old_keys, &new_keys) {
        let (tag, old_range, new_range) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        match regions.last_mut() {
            Some((old_region, new_region))
                if old_region.end == old_range.start && new_region.end == new_range.start =>
            {
                old_region.end = old_range.end;
                new_region.end = new_range.end;
            }
            _ => regions.push((old_range, new_range)),
        }
    }
    let regions = join_split_insertions(regions, &old_keys, &new_keys);

    Ok(regions
        .into_iter()
        .map(|(old_range, new_range)| {
            let new_range = if old_range.is_empty() {
                slide_insertion(&new, after, new_range, granularity)
            } else {
                new_range
            };
            let removed_text = if old_range.is_empty() {
                String::new()
            } else {
                before[old[old_range.start].start..old[old_range.end - 1].end].to_string()
            };
            let removed_lead = if old_range.is_empty() || !new_range.is_empty() {
                String::new()
            } else {
                let lead_start = old_range
                    .start
                    .checked_sub(1)
                    .map_or(0, |index| old[index].end);
                before[lead_start..old[old_range.start].start].to_string()
            };
            let context = |range: std::ops::Range<usize>| -> Vec<String> {
                new[range].iter().map(|unit| unit.text.clone()).collect()
            };
            Run {
                before: context(new_range.start.saturating_sub(CONTEXT)..new_range.start),
                after: context(new_range.end..(new_range.end + CONTEXT).min(new.len())),
                inserted: context(new_range),
                removed_text,
                removed_lead,
            }
        })
        .collect())
}

/// Equal units a split insertion may straddle: the diff's coincidental
/// matches inside an inserted block are short (a `(`, a `let`).
const MAX_SPLIT_GAP: usize = 4;

type Region = (std::ops::Range<usize>, std::ops::Range<usize>);

/// Myers can split one insertion in two by matching a few old units
/// against units inside the inserted block, and the split run's span can
/// then reach past a comment. When two pure insertions are separated by at
/// most [`MAX_SPLIT_GAP`] equal units and the whole window is also a single
/// insertion (the old gap is a prefix plus a suffix of it), join them, so
/// [`slide_insertion`] can place the insertion whole.
fn join_split_insertions(regions: Vec<Region>, old: &[&str], new: &[&str]) -> Vec<Region> {
    let mut joined: Vec<Region> = Vec::new();
    for (old_range, new_range) in regions {
        if let Some((last_old, last_new)) = joined.last_mut()
            && last_old.start == last_old.end
            && old_range.is_empty()
            && old_range.start - last_old.start <= MAX_SPLIT_GAP
        {
            let gap = &old[last_old.start..old_range.start];
            let window = &new[last_new.start..new_range.end];
            let prefix = gap.iter().zip(window).take_while(|(a, b)| a == b).count();
            let suffix = gap
                .iter()
                .rev()
                .zip(window.iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            if prefix + suffix >= gap.len() {
                // Keep the gap's first `kept` units before the insertion.
                let kept = gap.len() - suffix.min(gap.len());
                let start = last_new.start + kept;
                let at = last_old.start + kept;
                *last_old = at..at;
                *last_new = start..start + (window.len() - gap.len());
                continue;
            }
        }
        joined.push((old_range, new_range));
    }
    joined
}

/// A pure insertion can often be shifted by whole units without changing
/// the result (inserting `let a; let` after `let` equals inserting
/// `let a;` before it). Reverting deletes from the previous unit's end to
/// the run's last unit, so pick the alignment whose span holds no comment:
/// comments aren't units, and one caught in the span would be deleted too.
fn slide_insertion(
    new: &[Unit],
    after: &str,
    range: std::ops::Range<usize>,
    granularity: Granularity,
) -> std::ops::Range<usize> {
    if granularity == Granularity::Lines || range.is_empty() {
        return range;
    }
    let mut candidates = vec![range.clone()];
    let mut left = range.clone();
    while left.start > 0 && new[left.start - 1].text == new[left.end - 1].text {
        left = left.start - 1..left.end - 1;
        candidates.push(left.clone());
    }
    let mut right = range.clone();
    while right.end < new.len() && new[right.end].text == new[right.start].text {
        right = right.start + 1..right.end + 1;
        candidates.push(right.clone());
    }
    let has_comment = |candidate: &std::ops::Range<usize>| {
        let start = candidate
            .start
            .checked_sub(1)
            .map_or(0, |index| new[index].end);
        let span = &after[start..new[candidate.end - 1].end];
        span.contains("//") || span.contains("/*")
    };
    let inserted_has_comment = has_comment(&range) && candidates.iter().all(has_comment);
    if inserted_has_comment {
        // Every alignment spans a comment: it's part of what was inserted.
        return range;
    }
    candidates
        .into_iter()
        .filter(|candidate| !has_comment(candidate))
        .min_by_key(|candidate| candidate.start.abs_diff(range.start))
        .unwrap_or(range)
}

/// `current` with every run reverted, or why some can't be.
fn revert_runs(
    file: &str,
    current: &str,
    runs: &[Run],
    granularity: Granularity,
) -> Result<String, Vec<String>> {
    let units =
        units(current, granularity).map_err(|message| vec![format!("{file}: {message}")])?;
    let keys: Vec<&str> = units.iter().map(|unit| unit.text.as_str()).collect();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut drift = Vec::new();
    for run in runs {
        match locate(&keys, run) {
            Ok(position) => edits.push(edit_for(current, &units, run, position, granularity)),
            Err(reason) => drift.push(format!("{file}: {reason} (`{}`)", preview(run))),
        }
    }
    edits.sort_by_key(|(start, ..)| *start);
    if edits.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        drift.push(format!("{file}: recorded regions now overlap"));
    }
    if !drift.is_empty() {
        return Err(drift);
    }
    let mut output = current.to_string();
    for (start, end, text) in edits.into_iter().rev() {
        output.replace_range(start..end, &text);
    }
    Ok(output)
}

/// Where a run is now: the index of its first inserted unit (or, for a
/// pure deletion, the index it would be re-inserted before).
fn locate(keys: &[&str], run: &Run) -> Result<usize, String> {
    let score = |position: usize, len: usize| -> usize {
        let before = run
            .before
            .iter()
            .rev()
            .zip(keys[..position].iter().rev())
            .take_while(|(want, have)| want.as_str() == **have)
            .count();
        let after = run
            .after
            .iter()
            .zip(keys[position + len..].iter())
            .take_while(|(want, have)| want.as_str() == **have)
            .count();
        before + after
    };
    let len = run.inserted.len();
    let candidates: Vec<(usize, usize)> = if len == 0 {
        // A pure deletion: the full context must still be adjacent.
        let needed = run.before.len() + run.after.len();
        (0..=keys.len())
            .map(|position| (position, score(position, 0)))
            .filter(|(_, score)| *score == needed && needed > 0)
            .collect()
    } else {
        (0..keys.len().saturating_sub(len - 1))
            .filter(|position| {
                keys[*position..*position + len]
                    .iter()
                    .zip(&run.inserted)
                    .all(|(have, want)| *have == want.as_str())
            })
            .map(|position| (position, score(position, len)))
            .collect()
    };
    let best = candidates.iter().map(|(_, score)| *score).max();
    let top: Vec<usize> = candidates
        .iter()
        .filter(|(_, score)| Some(*score) == best)
        .map(|(position, _)| *position)
        .collect();
    match top.as_slice() {
        [position] => Ok(*position),
        [] => Err("the inserted code is no longer there (edited or removed)".to_string()),
        _ => Err("the inserted code appears more than once; can't tell which".to_string()),
    }
}

/// The `(start, end, replacement)` byte edit that reverts `run` at
/// `position`.
fn edit_for(
    current: &str,
    units: &[Unit],
    run: &Run,
    position: usize,
    granularity: Granularity,
) -> (usize, usize, String) {
    let len = run.inserted.len();
    if len == 0 {
        let at = match granularity {
            Granularity::Tokens => position.checked_sub(1).map_or_else(
                || units.get(position).map_or(current.len(), |unit| unit.start),
                |index| units[index].end,
            ),
            Granularity::Lines => units.get(position).map_or(current.len(), |unit| unit.start),
        };
        return (at, at, format!("{}{}", run.removed_lead, run.removed_text));
    }
    let first = &units[position];
    let last = &units[position + len - 1];
    if run.removed_text.is_empty() {
        // A pure insertion: also take the whitespace that separated it from
        // the previous unit, so no blank line or stray space is left.
        let start = match granularity {
            Granularity::Tokens => position
                .checked_sub(1)
                .map_or(first.start, |index| units[index].end),
            Granularity::Lines => first.start,
        };
        (start, last.end, String::new())
    } else {
        (first.start, last.end, run.removed_text.clone())
    }
}

fn preview(run: &Run) -> String {
    let text = if run.inserted.is_empty() {
        run.removed_text.clone()
    } else {
        run.inserted.join(" ")
    };
    text.chars().take(60).collect::<String>().replace('\n', " ")
}

/// FNV-1a, hex: a stable content fingerprint for created files.
#[must_use]
pub fn hash(text: &str) -> String {
    let mut value: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{value:016x}")
}

#[cfg(test)]
mod tests {
    use super::join_split_insertions;

    #[test]
    fn a_split_insertion_is_joined_back_into_one() {
        // old `let ( x`; new `let a ( b ; let ( x`, as Myers split it:
        // `a` after the old `let`, the old `(` matched inside the block,
        // then `b ; let (` before `x`.
        let old = ["let", "(", "x"];
        let new = ["let", "a", "(", "b", ";", "let", "(", "x"];
        let split = vec![(1..1, 1..2), (2..2, 3..7)];
        assert_eq!(join_split_insertions(split, &old, &new), vec![(1..1, 1..6)]);
    }

    #[test]
    fn separate_insertions_stay_separate() {
        let old = ["a", "b", "c"];
        let new = ["a", "x", "b", "y", "c"];
        let split = vec![(1..1, 1..2), (2..2, 3..4)];
        assert_eq!(join_split_insertions(split.clone(), &old, &new), split);
    }
}
