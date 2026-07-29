//! UO serials: the identity an object has *on the wire*.
//!
//! The Ultima Online protocol addresses everything by a 32-bit serial, and the
//! client infers an object's category from the numeric range it falls in.
//! Mobiles and items therefore come from two disjoint pools and cannot be
//! renumbered freely. This is a hard protocol constraint, not a Sphere-ism.
//!
//! # Why this lives in `protocol` and not in `entities`
//!
//! A serial is a wire concept first: it is what a packet field *is*, and the
//! rule that decides whether `0x4000_0001` is a mobile or an item is the
//! client's, not the server's. The entity registry then borrows it as its
//! external identity — so `entities` depends on `protocol` for it, never the
//! other way round. Allocation, on the other hand, is a server policy and stays
//! in `entities` with the registry that needs it.

use std::fmt;

/// Lowest valid mobile serial. Zero is reserved.
pub const MOBILE_MIN: u32 = 0x0000_0001;
/// Highest valid mobile serial.
pub const MOBILE_MAX: u32 = 0x3FFF_FFFF;
/// Lowest valid item serial.
pub const ITEM_MIN: u32 = 0x4000_0000;
/// Highest valid item serial.
pub const ITEM_MAX: u32 = 0x7FFF_FFFF;

/// Which pool a [`Serial`] belongs to. The client derives this from the range.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum SerialKind {
    /// A mobile: player, NPC, or pet.
    Mobile,
    /// An item: anything not a mobile, including multis.
    Item,
}

/// A 32-bit object identity as understood by the UO client.
///
/// Construction is checked: a `Serial` always falls inside a valid pool, so
/// [`Serial::kind`] is total and never lies.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Serial(u32);

impl Serial {
    /// Build a serial from a raw wire value.
    ///
    /// Returns `None` for values outside both pools (`0`, and anything at or
    /// above `0x8000_0000`), which the client would not accept. A packet field
    /// that may legitimately be empty — the object a ground click did not hit,
    /// the target a `0xAA` clears — is an `Option<Serial>`, and this is how the
    /// wire's `0` and `0xFFFF_FFFF` become that `None`.
    #[inline]
    pub const fn new(raw: u32) -> Option<Self> {
        if raw >= MOBILE_MIN && raw <= ITEM_MAX {
            Some(Self(raw))
        } else {
            None
        }
    }

    /// The raw value to put on the wire.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Which pool this serial came from.
    #[inline]
    pub const fn kind(self) -> SerialKind {
        if self.0 <= MOBILE_MAX {
            SerialKind::Mobile
        } else {
            SerialKind::Item
        }
    }

    /// True if this serial addresses a mobile.
    #[inline]
    pub const fn is_mobile(self) -> bool {
        self.0 <= MOBILE_MAX
    }

    /// True if this serial addresses an item.
    #[inline]
    pub const fn is_item(self) -> bool {
        self.0 > MOBILE_MAX
    }
}

impl fmt::Debug for Serial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind() {
            SerialKind::Mobile => "Mobile",
            SerialKind::Item => "Item",
        };
        write!(f, "Serial({kind} 0x{:08X})", self.0)
    }
}

impl fmt::Display for Serial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:08X}", self.0)
    }
}

/// The raw value an absent serial goes out as.
///
/// Zero is what every reference writes for "no object": a `0xAA` that clears the
/// attack target, an effect with no source mobile, a ground click that hit no
/// object. It is deliberately not a [`Serial`] — nothing can be addressed by it.
pub(crate) const NO_SERIAL: u32 = 0;

/// The wire value of `serial`, or [`NO_SERIAL`] for nothing.
///
/// Every packet that can carry an empty object field goes through this, so the
/// choice of sentinel is made in one place rather than re-decided per encoder.
pub(crate) const fn raw_or_none(serial: Option<Serial>) -> u32 {
    match serial {
        Some(serial) => serial.raw(),
        None => NO_SERIAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_decides_kind() {
        assert_eq!(Serial::new(MOBILE_MIN).unwrap().kind(), SerialKind::Mobile);
        assert_eq!(Serial::new(MOBILE_MAX).unwrap().kind(), SerialKind::Mobile);
        assert_eq!(Serial::new(ITEM_MIN).unwrap().kind(), SerialKind::Item);
        assert_eq!(Serial::new(ITEM_MAX).unwrap().kind(), SerialKind::Item);
    }

    #[test]
    fn rejects_out_of_range() {
        assert_eq!(Serial::new(0), None);
        assert_eq!(Serial::new(ITEM_MAX + 1), None);
        assert_eq!(Serial::new(u32::MAX), None, "0xFFFFFFFF means 'nothing'");
    }

    #[test]
    fn an_absent_serial_goes_out_as_zero() {
        assert_eq!(raw_or_none(None), 0);
        assert_eq!(raw_or_none(Serial::new(0x2A)), 0x2A);
    }
}
