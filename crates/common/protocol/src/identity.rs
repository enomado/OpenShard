//! The three strings that identify a player, kept apart so they cannot be
//! mixed up at a call site: which account they log in as, which character
//! they play, and the plaintext password the client just sent (never to be
//! confused with the argon2 hash a store keeps instead).
//!
//! # Raw off the wire, validated once
//!
//! [`AccountName`], [`CharacterName`] and [`PlaintextPassword`] are the
//! trusted, already-good forms — what `[[accounts]]` in a config file names,
//! what a store keys its rows by. A `0x80`/`0x91`/`0x00`/`0xF8` packet carries
//! the *same-shaped* bytes but they are client input, not an invariant: a
//! client can send an empty name, a 30-byte field padded with garbage, or
//! anything else the wire format does not forbid. [`RawAccountName`],
//! [`RawCharacterName`] and [`RawPlaintextPassword`] are that unchecked form —
//! every client-to-server wire struct in [`crate::login`] and [`crate::world`]
//! carries the `Raw` type, and the only way to a trusted [`AccountName`] or
//! [`CharacterName`] is through the check that makes one, in
//! `openshard_login::Accounts::verify`/`create_character`. This is the same
//! shape as `openshard_config`'s `RawAccessLevel` vs. `AccessLevel`.
//!
//! `PlaintextPassword` and `RawPlaintextPassword` differ only for this
//! symmetry: a password has no "validated" long-lived form (it is hashed or
//! compared and then dropped), but it is client input all the same and its
//! type should say so.
//!
//! This crate carries no dependencies (it is the leaf everything else in
//! `common`/`server` builds on), so these types stay serde-free like every
//! other newtype here — a config or persistence crate that needs to
//! (de)serialize one does it with a small `serialize_with`/`deserialize_with`
//! pair at its own field, not by pulling serde in here.
//!
//! `.0` unwraps only where a value crosses into a different domain — the wire
//! codec, a SQL bind, a `HashMap` key — never in ordinary call-tree code.

use std::fmt;

/// An account name, as typed at login or written in `[[accounts]]`.
///
/// Login is case-insensitive — `"Admin"` and `"admin"` are the same account —
/// but this type does not fold case itself. Every layer that keys a map by
/// account name calls [`AccountName::normalized`] explicitly at the point it
/// builds the key, the same way `Serial`/`EntityId` never hide validation
/// inside a trait impl.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct AccountName(pub String);

impl AccountName {
    /// The case-folded form used to key a map, so `"Admin"` and `"admin"`
    /// collide on purpose instead of shadowing each other silently.
    pub fn normalized(&self) -> String {
        self.0.to_lowercase()
    }
}

impl fmt::Display for AccountName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// Test fixtures compare against string literals constantly; comparing the
// wrapper directly against `&str` keeps those assertions readable without
// reaching for `.0` at every call site. Construction still goes through
// `AccountName(...)` — this is read-only ergonomics, not `Deref`.
impl PartialEq<str> for AccountName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for AccountName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

// For call sites that construct one from a literal or an owned `String` —
// most usefully as an `impl Into<AccountName>` parameter, so a test fixture
// like `.with_account("admin", ...)` needs no wrapping at all while a real
// caller passing an already-typed `AccountName` still just moves it.
impl From<&str> for AccountName {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for AccountName {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<&AccountName> for AccountName {
    fn from(value: &AccountName) -> Self {
        value.clone()
    }
}

/// An account name exactly as a `0x80`/`0x91` packet carried it: whatever sat
/// in the fixed-width field, not yet checked for length or emptiness. See the
/// module docs — `Accounts::verify` is the only place this becomes a real
/// [`AccountName`].
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RawAccountName(pub String);

impl fmt::Display for RawAccountName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialEq<str> for RawAccountName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for RawAccountName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl From<&str> for RawAccountName {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for RawAccountName {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<&RawAccountName> for RawAccountName {
    fn from(value: &RawAccountName) -> Self {
        value.clone()
    }
}

/// A character name, as typed at creation and shown in the character list.
///
/// `Default` is the empty name, meaning an unused character-list slot — see
/// [`crate::login::CharacterEntry`].
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct CharacterName(pub String);

impl CharacterName {
    /// The case-folded form used to key a map. See [`AccountName::normalized`].
    pub fn normalized(&self) -> String {
        self.0.to_lowercase()
    }
}

impl fmt::Display for CharacterName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialEq<str> for CharacterName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for CharacterName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl From<&str> for CharacterName {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for CharacterName {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<&CharacterName> for CharacterName {
    fn from(value: &CharacterName) -> Self {
        value.clone()
    }
}

/// A character name exactly as a `0x00`/`0xF8` create-character packet
/// carried it: not yet trimmed, checked for length, or checked for
/// emptiness/duplication against the account. See the module docs —
/// `Accounts::create_character` is the only place this becomes a real
/// [`CharacterName`].
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct RawCharacterName(pub String);

impl fmt::Display for RawCharacterName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialEq<str> for RawCharacterName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for RawCharacterName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl From<&str> for RawCharacterName {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for RawCharacterName {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<&RawCharacterName> for RawCharacterName {
    fn from(value: &RawCharacterName) -> Self {
        value.clone()
    }
}

/// A password as an operator wrote it in `openshard.toml`: plaintext, not yet
/// hashed.
///
/// No `Display`, and `Debug` is hand-written to redact — a stray `{:?}` in a
/// log line is a credential leak.
#[derive(Clone, PartialEq, Eq)]
pub struct PlaintextPassword(pub String);

impl fmt::Debug for PlaintextPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PlaintextPassword(<redacted>)")
    }
}

impl From<&str> for PlaintextPassword {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for PlaintextPassword {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<&PlaintextPassword> for PlaintextPassword {
    fn from(value: &PlaintextPassword) -> Self {
        value.clone()
    }
}

/// A password exactly as a `0x80`/`0x91` packet carried it: plaintext, and —
/// unlike [`AccountName`]/[`CharacterName`] — never promoted to a "validated"
/// form, because a password has no long-lived trusted shape: it is hashed or
/// compared once and then dropped. It is still client input, so it gets its
/// own type rather than reusing [`PlaintextPassword`] — see the module docs.
///
/// The UO login packet's password field is plaintext inside encryption that
/// is trivially broken, so the protocol itself treats it as public on the
/// wire; `Debug` still redacts, because "public on the wire" is not a license
/// to put it in a log line.
#[derive(Clone, PartialEq, Eq)]
pub struct RawPlaintextPassword(pub String);

impl fmt::Debug for RawPlaintextPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RawPlaintextPassword(<redacted>)")
    }
}

impl From<&str> for RawPlaintextPassword {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for RawPlaintextPassword {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<&RawPlaintextPassword> for RawPlaintextPassword {
    fn from(value: &RawPlaintextPassword) -> Self {
        value.clone()
    }
}
