//! Musical instruments — which sounds each one makes.
//!
//! The bard skills all play something, and what it sounds like is a property of the
//! instrument *class*, not of the particular lute in your pack: ServUO's
//! `BaseInstrument` subclasses each pass a pair of sound ids to their base
//! constructor. So it is a core table keyed by graphic, the shape
//! [`crate::weapon`] and [`crate::armor`] use, and how worn a given instrument is
//! lives on the item as an [`Instrument`](crate::Instrument).

/// One instrument class: the graphic, and the two sounds it makes.
#[derive(Debug, Clone, Copy)]
pub struct InstrumentData {
    /// The item graphic this row describes.
    pub graphic: u16,
    /// What it sounds like played well — ServUO's `SuccessSound`.
    pub well: u16,
    /// And badly.
    pub badly: u16,
}

/// How many tunes a fresh instrument holds — ServUO's `InitMinUses`/`InitMaxUses`,
/// rolled between. A bard's instrument is a consumable, which is what stops
/// Provocation being free to attempt.
pub const INSTRUMENT_MIN_USES: u16 = 350;
/// The top of that range.
pub const INSTRUMENT_MAX_USES: u16 = 450;

/// The instrument row for an item graphic, or `None` for anything that is not one.
#[must_use]
pub fn instrument_data(graphic: u16) -> Option<&'static InstrumentData> {
    INSTRUMENTS.iter().find(|i| i.graphic == graphic)
}

/// The classic instruments, from `ServUO/Scripts/Items/Equipment/Instruments/*.cs`
/// — each row is that class's `: base(graphic, wellSound, badlySound)`.
#[rustfmt::skip]
static INSTRUMENTS: &[InstrumentData] = &[
    i(0x0EB1, 0x43, 0x44), // Harp
    i(0x0EB2, 0x45, 0x46), // Lap harp
    i(0x0EB3, 0x4C, 0x4D), // Lute
    i(0x0E9C, 0x38, 0x39), // Drums
    i(0x0E9D, 0x52, 0x53), // Tambourine
    i(0x0E9E, 0x52, 0x53), // Tambourine with tassels
];

/// A row, so the table reads as data.
const fn i(graphic: u16, well: u16, badly: u16) -> InstrumentData {
    InstrumentData { graphic, well, badly }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_instrument_has_its_own_pair_of_sounds() {
        // A lute and a drum are not interchangeable, and the pair is what makes a
        // failed performance audibly different from a good one.
        let lute = instrument_data(0x0EB3).expect("a lute");
        assert_eq!((lute.well, lute.badly), (0x4C, 0x4D));
        let drums = instrument_data(0x0E9C).expect("drums");
        assert_eq!((drums.well, drums.badly), (0x38, 0x39));
        assert!(instrument_data(0x0000).is_none());
    }

    #[test]
    fn no_two_instruments_share_a_graphic() {
        for (n, a) in INSTRUMENTS.iter().enumerate() {
            for b in &INSTRUMENTS[n + 1..] {
                assert_ne!(a.graphic, b.graphic, "duplicate 0x{:04X}", a.graphic);
            }
        }
    }
}
