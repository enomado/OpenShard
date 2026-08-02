//! `Equipconv.def`: which body-animation graphic a worn item draws as, on the
//! bodies where it is not the item's own `tiledata` `AnimID`.
//!
//! A worn item is not drawn from its own art. Its *default* picture is its
//! tiledata entry's `AnimID` field
//! ([`crate::tiledata::StaticTile::anim_id`]) — a plain shirt needs nothing
//! more, which is why an ordinary outfit has no entries here at all. This
//! table only overrides that default for the pairs where a body needs a
//! *different* picture — chiefly a race or gender variant of the same
//! garment — keyed by `(body, AnimID)` and not by the item's own art graphic.
//! Confirmed by reading `AnimationsLoader.ProcessEquipConvDef` and
//! `MobileView.AddItem` in ClassicUO, a BSD-2 reference client kept outside
//! this repository: `graphic = item.ItemData.AnimID`, then replaced with the
//! table's `newGraphic` only if `(body, AnimID)` is a key in it. A missing
//! pair is the ordinary case, not a failure — it means "the default already
//! draws right."
//!
//! Unlike every other file this crate reads, this one is text, not binary: a
//! line per entry, tab- or space-separated, `#` starts a comment, and there is
//! no record count to divide the file by — the same def-file shape as
//! `Body.def`/`Bodyconv.def`, neither of which this crate has needed yet.
//! Five columns per entry: `body`, the item's `AnimID`, the `AnimID` to draw
//! instead, a gump graphic (paperdoll icon, not read here — nothing in this
//! engine draws a paperdoll off this table yet), and a hue that applies only
//! when the wire hue is `0`.
//!
//! Not read here either: `mobtypes.txt`, which the reference client uses to
//! tell a human body from a gargoyle one and shift the resolved graphic
//! accordingly. Without it, a gargoyle's equipment resolves through the same
//! table a human's does and comes out wrong for the pairs a gargoyle actually
//! needs converted — a known gap, not a guess.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use openshard_protocol::wire::{Graphic, Hue};

/// What one `(body, item graphic)` pair resolves to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EquipConvEntry {
    /// The body-animation-space graphic to draw in place of the item's own.
    pub graphic: Graphic,
    /// Applied only where the item's own wire hue is [`Hue::NONE`] — the same
    /// rule a static's tiledata-driven hue follows.
    pub color: Hue,
}

/// The whole table, keyed by the pair a lookup always has both halves of.
#[derive(Clone, Debug, Default)]
pub struct EquipConv {
    entries: HashMap<(u16, u16), EquipConvEntry>,
}

/// `Equipconv.def` could not be read.
#[derive(Debug)]
#[non_exhaustive]
pub enum EquipConvError {
    /// The file could not be read.
    Read {
        /// Which file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
}

impl fmt::Display for EquipConvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for EquipConvError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
        }
    }
}

impl EquipConv {
    /// Read `Equipconv.def`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, EquipConvError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| EquipConvError::Read {
            path: path.to_owned(),
            source,
        })?;
        Ok(Self::parse(&text))
    }

    /// Parse text already in memory.
    ///
    /// A line this cannot make sense of — blank, comment-only, or short of
    /// its five columns — is skipped rather than refused: the file is
    /// hand-edited data with the same tolerance for stray lines every other
    /// def-file reader in a UO client extends it, and one malformed record
    /// should not cost every entry after it.
    pub fn parse(text: &str) -> Self {
        let mut entries = HashMap::new();
        for line in text.lines() {
            let line = match line.find('#') {
                Some(index) => &line[..index],
                None => line,
            };
            let mut columns = line.split_whitespace();
            let (Some(body), Some(graphic), Some(new_graphic), Some(_gump), Some(color)) = (
                columns.next(),
                columns.next(),
                columns.next(),
                columns.next(),
                columns.next(),
            ) else {
                continue;
            };
            let (Ok(body), Ok(graphic), Ok(new_graphic), Ok(color)) = (
                body.parse::<u16>(),
                graphic.parse::<u16>(),
                new_graphic.parse::<u16>(),
                color.parse::<u16>(),
            ) else {
                continue;
            };
            entries.insert(
                (body, graphic),
                EquipConvEntry {
                    graphic: Graphic(new_graphic),
                    color: Hue(color),
                },
            );
        }
        Self { entries }
    }

    /// What a worn item with this `AnimID`, on a mobile of this body, draws
    /// as instead of that `AnimID` — or `None` if the pair is not in the
    /// table, which is the ordinary case and means "the `AnimID` itself is
    /// already right for this body", not "draw nothing".
    pub fn resolve(&self, body: u16, item_anim_id: u16) -> Option<EquipConvEntry> {
        self.entries.get(&(body, item_anim_id)).copied()
    }

    /// How many pairs the table holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_entry() {
        let table = EquipConv::parse("400\t7017\t7005\t0\t0\n");
        let entry = table.resolve(400, 7017).expect("the pair above should resolve");
        assert_eq!(entry.graphic, Graphic(7005));
        assert_eq!(entry.color, Hue(0));
    }

    #[test]
    fn comments_and_blank_lines_are_not_entries() {
        let table = EquipConv::parse("# a comment\n\n   \n# 400 7017 7005 0 0\n");
        assert!(table.is_empty());
    }

    #[test]
    fn a_trailing_comment_on_a_data_line_is_stripped() {
        let table = EquipConv::parse("400\t7017\t7005\t0\t0\t# a robe\n");
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn a_short_line_is_skipped_not_refused() {
        let table = EquipConv::parse("400\t7017\t7005\n401\t7018\t7006\t0\t0\n");
        assert_eq!(table.len(), 1, "only the five-column line should parse");
        assert!(table.resolve(401, 7018).is_some());
    }

    #[test]
    fn an_unlisted_pair_resolves_to_nothing() {
        let table = EquipConv::parse("400\t7017\t7005\t0\t0\n");
        assert!(table.resolve(400, 9999).is_none());
        assert!(table.resolve(401, 7017).is_none());
    }
}
