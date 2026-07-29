//! Casting from the client — a spellbook or a macro asking to cast a spell.
//!
//! What the *effect* of a spell is — its mana, its reagents, its damage — is not
//! here and not the engine's: a script owns it, Sphere-scriptpack style. This is
//! only the request off the wire, so the server can hand "this player wants to
//! cast spell N" to the script and let the script's spell data do the rest.
//!
//! The modern client (ClassicUO, 7.x) casts from a spellbook through the extended
//! `0xBF` packet, subcommand `0x1C` — the shape read from ServUO's
//! `PacketHandlers.CastSpell`. There is an older text-command form (`0x12`) too;
//! this handles the one a modern client actually sends.

use crate::codec::PacketReader;
use crate::error::DecodeError;

/// `0xBF` subcommand `0x1C` — a spellbook (or macro) asking to cast a spell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CastSpellRequest {
    /// Which spell, zero-based. The wire carries it one-based (so `1` is the first
    /// spell); this is already decremented, matching how a script names spells.
    pub spell: u16,
}

impl CastSpellRequest {
    /// The subcommand that means "cast a spell". See
    /// [`ExtendedRequest`](crate::extended::ExtendedRequest), the single place
    /// that reads a `0xBF` envelope and picks a subcommand's body decoder by it.
    pub const SUBCOMMAND: u16 = 0x1C;

    /// Read the body, `reader` already past the id, length and subcommand.
    pub(crate) fn decode_body(reader: &mut PacketReader<'_>) -> Result<Self, DecodeError> {
        // ServUO's reading: a flag word, and when it is 1 the spellbook's serial
        // follows, then the one-based spell id. The spellbook is context the
        // engine does not need — only which spell.
        if reader.u16()? == 1 {
            let _spellbook = reader.u32()?;
        }
        let one_based = reader.u16()?;
        Ok(Self {
            spell: one_based.saturating_sub(1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extended::ExtendedRequest;

    /// Build a `0xBF.0x1C` cast packet the way the client does.
    fn cast_packet(spellbook: Option<u32>, one_based_spell: u16) -> Vec<u8> {
        let mut body = vec![0xBFu8, 0, 0]; // id + length patched below
        body.extend_from_slice(&0x1Cu16.to_be_bytes()); // subcommand
        match spellbook {
            Some(serial) => {
                body.extend_from_slice(&1u16.to_be_bytes());
                body.extend_from_slice(&serial.to_be_bytes());
            }
            None => body.extend_from_slice(&0u16.to_be_bytes()),
        }
        body.extend_from_slice(&one_based_spell.to_be_bytes());
        let len = u16::try_from(body.len()).unwrap();
        body[1..3].copy_from_slice(&len.to_be_bytes());
        body
    }

    #[test]
    fn a_spellbook_cast_names_its_spell_zero_based() {
        // The client sends the sixth spell as 6; the engine sees 5.
        let packet = cast_packet(Some(0x4000_0001), 6);
        let request = ExtendedRequest::decode(&packet).unwrap();
        assert_eq!(request, ExtendedRequest::Cast(CastSpellRequest { spell: 5 }));
    }

    #[test]
    fn a_macro_cast_carries_no_spellbook() {
        let packet = cast_packet(None, 1);
        let request = ExtendedRequest::decode(&packet).unwrap();
        assert_eq!(
            request,
            ExtendedRequest::Cast(CastSpellRequest { spell: 0 }),
            "the first spell, zero-based"
        );
    }
}
