//! The Double Precision Array File container that an SPK kernel is built on.
//!
//! An SPK file is a `DAF` file whose summaries describe segments: a time interval, a target, a
//! centre, a reference frame, a data type, and the word addresses of the segment's own data. The
//! format is public — NAIF's "DAF Required Reading" — and this module reads exactly the parts an
//! SPK file needs, refusing anything it does not understand rather than guessing.
//!
//! # Layout, as read here
//!
//! - **Record 1** is the file record: `DAF/SPK ` in the first eight bytes, the two summary sizes
//!   `ND` and `NI` as little-endian `i32` at bytes 8 and 12, the first and last summary record
//!   numbers `FWARD` and `BWARD` at bytes 76 and 80, and the binary format identifier at bytes 88
//!   to 95.
//! - **Records between a comment area and the free word** hold the summaries. Each begins with
//!   three doubles — next record, previous record, number of summaries in this record — because the
//!   control words are doubles on purpose, so they read the same on either endianness.
//! - Each summary is `ND` doubles followed by `NI` integers, stored as raw `f64` and `i32`.
//!
//! Records are 1024 bytes. Words are 8 bytes and are addressed from one.

use stkcraft_framework_core::{Error, Result};

/// The size of a DAF record, in bytes.
pub const RECORD_BYTES: usize = 1024;

/// The size of a DAF word, in bytes.
pub const WORD_BYTES: usize = 8;

/// One summary: the time interval it covers and its six integers.
///
/// The doubles are `[START, STOP]` and the integers are `[TARGET, CENTER, FRAME, TYPE, START,
/// END]`, which is what an SPK file stores; a non-SPK DAF would put something else there, which is
/// why the file's own identifier is checked before this type is built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    /// The segment's first epoch, seconds past J2000 TDB.
    pub start_et: f64,
    /// The segment's last epoch, seconds past J2000 TDB.
    pub stop_et: f64,
    /// `[target, centre, frame, type, start word, end word]`.
    pub ints: [i32; 6],
}

/// A parsed DAF file: its summaries, and the bytes they address.
#[derive(Debug)]
pub struct Daf {
    bytes: Vec<u8>,
    summaries: Vec<Summary>,
}

impl Daf {
    /// Parse an SPK file held in memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for a file that is not an SPK, that is not little-endian
    /// IEEE binary, or that declares a summary layout this reader does not implement; and
    /// [`Error::InvalidInput`] for a file whose records or addresses do not add up.
    #[allow(clippy::indexing_slicing)] // fixed-offset reads of a record whose length is checked
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < RECORD_BYTES {
            return Err(Error::invalid(
                "SPK file",
                "shorter than the file record; not a DAF file",
            ));
        }
        let identifier = &bytes[0..8];
        if identifier != b"DAF/SPK " {
            return Err(Error::unsupported(
                "this file is not an SPK kernel (it does not start with `DAF/SPK `)",
            ));
        }
        let format = &bytes[88..96];
        // `LTL-IEEE` is little-endian IEEE binary. `BIG-IEEE` would need a byte swap on every
        // value and `FUR-LTL`/`FUR-BIG` are formatted doubles; none of them is read here, and
        // silently reinterpreting one as the other would produce a plausible wrong trajectory.
        if format != b"LTL-IEEE" {
            return Err(Error::unsupported(
                "only little-endian IEEE SPK files are read; this one declares a different format",
            ));
        }
        let nd = i32_at(&bytes, 8)?;
        let ni = i32_at(&bytes, 12)?;
        let fward = i32_at(&bytes, 76)?;
        let bward = i32_at(&bytes, 80)?;
        // An SPK summary is exactly two doubles and six integers.
        if nd != 2 || ni != 6 {
            return Err(Error::invalid(
                "SPK file",
                format!("an SPK summary is 2 doubles and 6 integers; this declares {nd} and {ni}"),
            ));
        }
        if fward < 2 || bward < fward {
            return Err(Error::invalid(
                "SPK file",
                "the summary record bounds are not a forward range",
            ));
        }
        let last_record = bytes.len() / RECORD_BYTES;
        if usize::try_from(bward).unwrap_or(usize::MAX) > last_record {
            return Err(Error::invalid(
                "SPK file",
                "a summary record lies past the end of the file",
            ));
        }

        // The summary records are a linked list, not a contiguous range: a kernel with many
        // segments (Saturn's, with nine satellites and the planet) puts them in records at either
        // end of the file with the data in between. Following `NEXT` from `FWARD` is the format's
        // own answer; iterating `FWARD..=BWARD` reads data records as summaries and, with luck,
        // only fails on a nonsense count.
        let mut summaries = Vec::new();
        let mut record = fward;
        let mut visited = 0_usize;
        while record != 0 {
            visited += 1;
            if visited > 1024 {
                return Err(Error::invalid(
                    "SPK file",
                    "the summary records form a cycle or a very long chain",
                ));
            }
            // Addresses come from the file. On a 32-bit target the multiply can overflow, and an
            // overflow must be an error rather than a wrapped offset that reads the wrong record.
            let record_number = usize::try_from(record).map_err(|_| {
                Error::invalid("SPK file", "a summary record address is not positive")
            })?;
            let offset = record_number
                .checked_sub(1)
                .and_then(|index| index.checked_mul(RECORD_BYTES))
                .ok_or_else(|| {
                    Error::invalid("SPK file", "a summary record address is too large")
                })?;
            let end = offset.checked_add(RECORD_BYTES).ok_or_else(|| {
                Error::invalid("SPK file", "a summary record address is too large")
            })?;
            let record_bytes = bytes
                .get(offset..end)
                .ok_or_else(|| Error::invalid("SPK file", "a summary record is truncated"))?;
            let count = count_of_summaries(record_bytes)?;
            for index in 0..count {
                summaries.push(summary_at(record_bytes, index)?);
            }
            record = next_summary_record(record_bytes)?;
        }
        Ok(Self { bytes, summaries })
    }

    /// The file's summaries.
    #[must_use]
    pub fn summaries(&self) -> &[Summary] {
        &self.summaries
    }

    /// The bytes of the file, for a reader that knows how a segment is laid out.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The `count`-th word of the file, one-based, as a double.
    ///
    /// # Errors
    ///
    /// An address outside the file is an error rather than a panic: the addresses come from the
    /// file itself, so a corrupt file must fail readably.
    pub fn word(&self, word: i32) -> Result<f64> {
        let start = word_offset(word)?;
        let slice = self
            .bytes
            .get(start..start + WORD_BYTES)
            .ok_or_else(|| Error::invalid("SPK file", "a segment address is past the end"))?;
        let mut eight = [0_u8; WORD_BYTES];
        eight.copy_from_slice(slice);
        Ok(f64::from_le_bytes(eight))
    }

    /// A 32-bit integer stored in the low half of a word.
    ///
    /// The SPK segment layout puts a segment's record count and coefficient count in words, as
    /// four-byte integers; the other four bytes of the word are unused.
    ///
    /// # Errors
    ///
    /// As [`Self::word`].
    pub fn int(&self, word: i32) -> Result<i32> {
        let start = word_offset(word)?;
        let slice = self
            .bytes
            .get(start..start + 4)
            .ok_or_else(|| Error::invalid("SPK file", "a segment field is past the end"))?;
        let mut four = [0_u8; 4];
        four.copy_from_slice(slice);
        Ok(i32::from_le_bytes(four))
    }
}

/// The byte offset of a one-based word address.
fn word_offset(word: i32) -> Result<usize> {
    let one_based = usize::try_from(word)
        .map_err(|_| Error::invalid("SPK file", "a segment address is not positive"))?;
    if one_based == 0 {
        return Err(Error::invalid("SPK file", "a segment address is zero"));
    }
    one_based
        .checked_sub(1)
        .and_then(|index| index.checked_mul(WORD_BYTES))
        .ok_or_else(|| Error::invalid("SPK file", "a segment address is too large"))
}

fn i32_at(bytes: &[u8], offset: usize) -> Result<i32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| Error::invalid("SPK file", "a file record offset is too large"))?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| Error::invalid("SPK file", "the file record is truncated"))?;
    let mut four = [0_u8; 4];
    four.copy_from_slice(slice);
    Ok(i32::from_le_bytes(four))
}

/// The record number of the next summary record, or zero at the end of the chain.
fn next_summary_record(record: &[u8]) -> Result<i32> {
    let slice = record
        .get(0..8)
        .ok_or_else(|| Error::invalid("SPK file", "a summary record is truncated"))?;
    let mut eight = [0_u8; 8];
    eight.copy_from_slice(slice);
    let next = f64::from_le_bytes(eight);
    if !next.is_finite() || next < 0.0 || next > f64::from(i32::MAX) || next.fract() != 0.0 {
        return Err(Error::invalid(
            "SPK file",
            "a summary record's next-record pointer is not a record number",
        ));
    }
    Ok(next as i32)
}

/// How many summaries a summary record holds.
#[allow(clippy::indexing_slicing)] // a fixed 24-byte control-word prefix
fn count_of_summaries(record: &[u8]) -> Result<usize> {
    let slice = record
        .get(16..24)
        .ok_or_else(|| Error::invalid("SPK file", "a summary record is truncated"))?;
    let mut eight = [0_u8; 8];
    eight.copy_from_slice(slice);
    let count = f64::from_le_bytes(eight);
    if !(count.is_finite() && count >= 0.0) {
        return Err(Error::invalid(
            "SPK file",
            "a summary record declares a nonsense summary count",
        ));
    }
    // A record cannot hold more summaries than fit: 24 bytes of control words, then 40 per
    // summary. A count past that is a corrupt file, and allocating on it would be the bug an
    // attacker would reach for.
    let maximum = (RECORD_BYTES - 24) / 40;
    let count = count as usize;
    if count > maximum {
        return Err(Error::invalid(
            "SPK file",
            "a summary record declares more summaries than a record can hold",
        ));
    }
    Ok(count)
}

/// Read summary `index` from a summary record.
#[allow(clippy::indexing_slicing)] // the same fixed layout, already bounds-checked above
fn summary_at(record: &[u8], index: usize) -> Result<Summary> {
    let offset = 24 + index * 40;
    let doubles = record
        .get(offset..offset + 16)
        .ok_or_else(|| Error::invalid("SPK file", "a summary is truncated"))?;
    let ints = record
        .get(offset + 16..offset + 40)
        .ok_or_else(|| Error::invalid("SPK file", "a summary is truncated"))?;
    let mut start_bytes = [0_u8; 8];
    start_bytes.copy_from_slice(&doubles[0..8]);
    let mut stop_bytes = [0_u8; 8];
    stop_bytes.copy_from_slice(&doubles[8..16]);
    let mut fields = [0_i32; 6];
    for (slot, chunk) in fields.iter_mut().zip(ints.chunks(4)) {
        let mut four = [0_u8; 4];
        four.copy_from_slice(chunk);
        *slot = i32::from_le_bytes(four);
    }
    Ok(Summary {
        start_et: f64::from_le_bytes(start_bytes),
        stop_et: f64::from_le_bytes(stop_bytes),
        ints: fields,
    })
}

// The tests use `expect` freely: a failed expectation there is the report.
#[cfg(test)]
#[expect(clippy::expect_used)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_an_spk_is_refused() {
        let mut bytes = vec![0_u8; RECORD_BYTES * 2];
        bytes[0..8].copy_from_slice(b"DAF/CK  ");
        let error = Daf::parse(bytes).expect_err("a CK file is not an SPK");
        assert!(matches!(error, Error::Unsupported { .. }), "{error:?}");
    }

    #[test]
    fn a_file_too_short_to_hold_a_record_is_refused() {
        assert!(Daf::parse(vec![0_u8; 100]).is_err());
        assert!(Daf::parse(Vec::new()).is_err());
    }

    #[test]
    fn a_big_endian_or_formatted_file_is_refused_rather_than_reinterpreted() {
        // The failure this guards against is quiet: byte-swapped doubles give a trajectory that
        // looks like a trajectory.
        let mut bytes = spk_header();
        bytes[88..96].copy_from_slice(b"BIG-IEEE");
        assert!(Daf::parse(bytes).is_err());
    }

    /// A file record with everything this reader needs, and no summaries.
    fn spk_header() -> Vec<u8> {
        let mut bytes = vec![0_u8; RECORD_BYTES * 2];
        bytes[0..8].copy_from_slice(b"DAF/SPK ");
        bytes[8..12].copy_from_slice(&2_i32.to_le_bytes());
        bytes[12..16].copy_from_slice(&6_i32.to_le_bytes());
        // One summary record, which is also the last one.
        bytes[76..80].copy_from_slice(&2_i32.to_le_bytes());
        bytes[80..84].copy_from_slice(&2_i32.to_le_bytes());
        bytes[88..96].copy_from_slice(b"LTL-IEEE");
        bytes
    }

    #[test]
    fn an_empty_spk_parses_to_no_summaries() {
        let daf = Daf::parse(spk_header()).expect("a header with no summaries is valid");
        assert_eq!(daf.summaries().len(), 0);
    }

    #[test]
    fn a_summary_is_read_as_two_doubles_and_six_integers() {
        let mut bytes = spk_header();
        let record = &mut bytes[RECORD_BYTES..RECORD_BYTES * 2];
        // next, previous, count — as doubles.
        record[0..8].copy_from_slice(&0.0_f64.to_le_bytes());
        record[8..16].copy_from_slice(&0.0_f64.to_le_bytes());
        record[16..24].copy_from_slice(&1.0_f64.to_le_bytes());
        record[24..32].copy_from_slice(&(-4_734_072_000.0_f64).to_le_bytes());
        record[32..40].copy_from_slice(&(4_735_368_000.0_f64).to_le_bytes());
        for (index, value) in [499_i32, 0, 1, 2, 8065, 610_868].iter().enumerate() {
            let start = 40 + index * 4;
            record[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        let daf = Daf::parse(bytes).expect("a one-summary file");
        let summary = daf.summaries().first().expect("one summary");
        assert_eq!(summary.ints, [499, 0, 1, 2, 8065, 610_868]);
        assert!((summary.start_et - -4_734_072_000.0).abs() < 1e-6);
    }

    #[test]
    fn a_summary_count_beyond_what_a_record_holds_is_refused() {
        let mut bytes = spk_header();
        bytes[RECORD_BYTES + 16..RECORD_BYTES + 24].copy_from_slice(&1.0e9_f64.to_le_bytes());
        assert!(Daf::parse(bytes).is_err());
    }

    #[test]
    fn a_word_address_outside_the_file_is_an_error_not_a_panic() {
        let daf = Daf::parse(spk_header()).expect("valid header");
        assert!(daf.word(0).is_err());
        assert!(daf.word(-5).is_err());
        assert!(daf.word(i32::MAX).is_err());
        assert!(daf.int(i32::MAX).is_err());
    }
}
