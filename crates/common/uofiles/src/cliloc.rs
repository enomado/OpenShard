//! `Cliloc.enu`: the client's own text table, looked up by number.
//!
//! A gump's `{ xmfhtmlgump }`/`{ xmfhtmlgumpcolor }`/`{ xmfhtmltok }` elements
//! (`gump::Element::Localized`) carry no text on the wire at all — only a
//! number — because the string is assumed to already be sitting in every
//! client's own install. That is what this file is: roughly forty thousand
//! numbered English sentences, most of them plain and a few carrying
//! `~1_val~`-style argument slots this reader does not resolve (see
//! [`Cliloc::get`]).
//!
//! # Layout
//!
//! ```text
//!   header   int32 + int16, unread                    6 bytes
//!   record   int32 number, u8 flag, i16 length, text   8 + length bytes
//! ```
//!
//! Records run back to back to the end of the file, in no particular number
//! order — a lookup is a table built once, not a scan per call.
//!
//! One real-client wrinkle is deliberately not handled: a `Cliloc.enu` whose
//! fourth byte is `0x8E` is BWT-compressed (a client newer than this project
//! targets). That file is rejected rather than silently read as garbage — see
//! [`ClilocError::Compressed`].

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// `Cliloc.enu` could not be read.
#[derive(Debug)]
#[non_exhaustive]
pub enum ClilocError {
    /// The file could not be read.
    Read {
        /// Which file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The fourth byte is `0x8E`: a BWT-compressed table from a client newer
    /// than this project targets. Rejected rather than read as noise.
    Compressed {
        /// Which file.
        path: PathBuf,
    },
}

impl fmt::Display for ClilocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Compressed { path } => {
                write!(f, "{} is BWT-compressed, which this reader does not decode", path.display())
            }
        }
    }
}

impl std::error::Error for ClilocError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Compressed { .. } => None,
        }
    }
}

/// Every numbered string the client's own `Cliloc.enu` holds.
#[derive(Clone, Default)]
pub struct Cliloc {
    entries: HashMap<u32, String>,
}

impl fmt::Debug for Cliloc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cliloc").field("entries", &self.entries.len()).finish()
    }
}

impl Cliloc {
    /// Read `Cliloc.enu`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ClilocError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| ClilocError::Read {
            path: path.to_owned(),
            source,
        })?;
        if bytes.get(3) == Some(&0x8E) {
            return Err(ClilocError::Compressed { path: path.to_owned() });
        }
        Ok(Self::parse(&bytes))
    }

    /// Parse bytes already in memory. Never fails: a truncated record simply
    /// ends the table where the file did, the same tolerance
    /// [`crate::gumpart`]'s reader gives a cut-off entry.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Self {
        let mut entries = HashMap::new();
        // 4-byte header + 2-byte header, neither read by anything real.
        let Some(mut rest) = bytes.get(6..) else {
            return Self { entries };
        };
        while rest.len() >= 4 + 1 + 2 {
            let number = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
            let length = u16::from_le_bytes([rest[5], rest[6]]) as usize;
            let Some(text) = rest.get(7..7 + length) else {
                break;
            };
            entries.insert(number, String::from_utf8_lossy(text).into_owned());
            rest = &rest[7 + length..];
        }
        Self { entries }
    }

    /// How many strings this table holds.
    #[must_use]
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// The string a cliloc number names, or `None` past the table's end.
    ///
    /// **No argument substitution.** ServUO's own `~1_val~` slots (an item's
    /// name, a count) are left in the text verbatim — the caller travelled no
    /// arguments to fill them with, because nothing here carries them over
    /// the wire (that is the whole point of sending a number and not a
    /// string). A sentence with no slots — the common case for a plain
    /// dialog's title and buttons — reads exactly as authored.
    #[must_use]
    pub fn get(&self, number: u32) -> Option<&str> {
        self.entries.get(&number).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(number: u32, text: &str) -> Vec<u8> {
        let mut out = number.to_le_bytes().to_vec();
        out.push(0); // flag, unread
        out.extend_from_slice(&(text.len() as u16).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
        out
    }

    #[test]
    fn reads_records_past_the_header() {
        let mut bytes = vec![0u8; 6]; // the unread header
        bytes.extend(record(1_011_022, "Resurrection"));
        bytes.extend(record(1_011_011, "CONTINUE"));

        let table = Cliloc::parse(&bytes);
        assert_eq!(table.count(), 2);
        assert_eq!(table.get(1_011_022), Some("Resurrection"));
        assert_eq!(table.get(1_011_011), Some("CONTINUE"));
        assert_eq!(table.get(1), None, "a number never written is absent, not a panic");
    }

    #[test]
    fn a_truncated_record_ends_the_table_rather_than_panicking() {
        let mut bytes = vec![0u8; 6];
        bytes.extend(record(1, "whole"));
        bytes.extend_from_slice(&[9, 0, 0, 0, 0, 20, 0]); // a length claiming 20 bytes it does not have
        let table = Cliloc::parse(&bytes);
        assert_eq!(table.get(1), Some("whole"), "the good record before the cut still reads");
        assert_eq!(table.count(), 1);
    }

    #[test]
    fn an_empty_file_is_an_empty_table() {
        assert_eq!(Cliloc::parse(&[]).count(), 0);
        assert_eq!(Cliloc::parse(&[0u8; 6]).count(), 0, "header only, no records");
    }

    #[test]
    fn a_compressed_file_is_refused_by_its_own_marker() {
        let mut bytes = vec![0u8; 8];
        bytes[3] = 0x8E;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("openshard-cliloc-test-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Cliloc.enu");
        std::fs::write(&path, &bytes).unwrap();
        let error = Cliloc::load(&path).unwrap_err();
        assert!(matches!(error, ClilocError::Compressed { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
