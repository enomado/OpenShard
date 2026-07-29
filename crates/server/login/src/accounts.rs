//! Who is allowed in.

use std::collections::HashMap;

use openshard_protocol::access::AccessLevel;
use openshard_protocol::identity::{
    AccountName, CharacterName, PlaintextPassword, RawAccountName, RawCharacterName,
    RawPlaintextPassword,
};
use openshard_protocol::login::{
    CharacterEntry, DenyReason, ACCOUNT_NAME_LENGTH, CHARACTER_NAME_LENGTH, MIN_CHARACTER_SLOTS,
    PASSWORD_LENGTH,
};

use crate::password;

/// Somewhere accounts live.
///
/// A trait because the in-memory store below is a placeholder: real shards keep
/// accounts in a database — SQLite or PostgreSQL — and the login state machine
/// must not care which.
///
/// # Implementors must hash
///
/// The UO protocol sends passwords in plaintext — there is no challenge, no
/// nonce, nothing. That is the client's fault and cannot be fixed server-side.
/// What *can* be fixed is what happens next: an implementation of this trait
/// must compare against a slow password hash (argon2, bcrypt, scrypt) and must
/// never persist the plaintext. [`verify`](Accounts::verify) taking the
/// plaintext is unavoidable; storing it is not.
///
/// # Parameters take `impl Into<...>`
///
/// Not `&str`: an account name, a character name and a password are three
/// different things, and threading bare strings through this trait is exactly
/// how a caller ends up passing one where another belongs. Taking
/// `impl Into<AccountName>` etc. keeps that impossible for typed callers while
/// still letting a test fixture pass a string literal directly — [`AccountName`]
/// and friends impl `From<&str>` for exactly that.
pub trait Accounts {
    /// Check a name and password.
    ///
    /// Takes the *raw*, not-yet-validated wire types — see
    /// `openshard_protocol::identity`'s module docs — and on success returns
    /// the [`AccountName`] they turned out to name: the only way to a
    /// validated account name is through this check (or trusted config
    /// seeding via [`DevAccounts::with_account`]).
    ///
    /// Returns the reason on failure so the caller can log it. What the client
    /// is told is a separate decision — see [`DenyReason::wire_code`].
    fn verify(
        &self,
        account: impl Into<RawAccountName>,
        password: impl Into<RawPlaintextPassword>,
    ) -> Result<AccountName, DenyReason>;

    /// The characters on an account, in slot order.
    ///
    /// Empty for an account with none; the 0xA9 encoder pads the list out.
    fn characters(&self, account: impl Into<AccountName>) -> Vec<CharacterEntry>;

    /// Create a character in the first free slot and return its slot index
    /// together with the [`CharacterName`] the raw name validated to.
    ///
    /// The client sends this as `0x00`/`0xF8` on the game connection, after it
    /// is already authenticated, so the account is expected to exist. Failure
    /// modes map to the codes the client can render: a full account is
    /// [`DenyReason::TooManyCharacters`], and an empty, overlong or duplicate
    /// name is [`DenyReason::BadCharacter`].
    ///
    /// Returning the validated name (not just the slot) matters: this is the
    /// only place a [`RawCharacterName`] becomes a real `CharacterName` (see
    /// `openshard_protocol::identity`'s module docs), so a caller that needs
    /// the character's name after creation — to enter it into the world —
    /// takes it from here instead of re-deriving it from the raw input.
    ///
    /// Takes `&mut self` because it writes: a real store persists here, and the
    /// dev store keeps it in memory for the life of the process, which is enough
    /// for a freshly created character to show up in the list on reconnect.
    ///
    /// Unlike the rest of this trait, `account` and `name` are concrete types,
    /// not `impl Into<...>`: this call is the raw-to-validated seam itself, so
    /// the caller names `RawCharacterName` explicitly at the call site instead
    /// of letting a blanket `Into` paper over it.
    fn create_character(
        &mut self,
        account: AccountName,
        name: RawCharacterName,
    ) -> Result<(u32, CharacterName), DenyReason>;

    /// Delete the character in a slot and return its name.
    ///
    /// The client sends this as `0x83` on the game connection. Slots are
    /// positional: removing one shifts the later slots down, which is exactly
    /// what the `0x86` resend that follows expects. An out-of-range or empty slot
    /// is [`DenyReason::BadCharacter`] — the caller maps it to the client's
    /// delete-reject code. Whether the character may be deleted *at all* (it is
    /// not being played, it is old enough) is the caller's to check: this store
    /// does not know who is in the world.
    fn delete_character(
        &mut self,
        account: impl Into<AccountName>,
        slot: u32,
    ) -> Result<CharacterName, DenyReason>;

    /// The authority the account's characters play with — what staff commands
    /// they may run. Defaults to [`AccessLevel::Player`] so a store that has no
    /// notion of staff grants none, which is the safe direction to be wrong in.
    /// An unknown account is a player, not an error: this is asked after login,
    /// about an account already verified, and the answer only ever *withholds*
    /// authority.
    fn access_level(&self, _account: impl Into<AccountName>) -> AccessLevel {
        AccessLevel::Player
    }
}

/// One account in the dev store.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DevAccount {
    /// The credential — an argon2 PHC hash, never plaintext. See [`password`].
    pub credential: String,
    /// Whether logins are refused.
    pub blocked: bool,
    /// The characters on this account.
    pub characters: Vec<CharacterEntry>,
    /// The authority this account's characters play with.
    pub access: AccessLevel,
}

/// An in-memory account store.
///
/// The credentials it holds are argon2 hashes, not plaintext — the plaintext a
/// config file or a login packet carries is hashed on the way in and never
/// kept. The store itself is in memory: the server loads it from the persistent
/// [`Store`](openshard_persistence::Store) at boot and seeds it from config, and
/// write-through to the database happens off the tick. A test can still spin one
/// up in one line with [`with_account`](Self::with_account).
#[derive(Clone, Default, Debug)]
pub struct DevAccounts {
    /// Keyed by lowercased name — the client does not preserve case reliably
    /// and players do not expect it to matter.
    accounts: HashMap<String, DevAccount>,
}

impl DevAccounts {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an account with a plaintext password, which is hashed before storage.
    ///
    /// For config seeding and tests, where the password is known in the clear.
    /// An account loaded from the store already carries a hash and comes in
    /// through [`with_credential`](Self::with_credential) instead.
    pub fn with_account(
        self,
        account: impl Into<AccountName>,
        password: impl Into<PlaintextPassword>,
    ) -> Self {
        let credential = password::hash(&password.into());
        self.with_credential(account, &credential)
    }

    /// Add an account with an already-hashed credential and no characters.
    ///
    /// The path a stored account takes at boot: its PHC hash is loaded as-is,
    /// never re-hashed. A blank credential (which verifies against nothing)
    /// stands for an account row with no password set.
    pub fn with_credential(mut self, account: impl Into<AccountName>, credential: &str) -> Self {
        self.accounts.insert(
            account.into().normalized(),
            DevAccount {
                credential: credential.to_owned(),
                blocked: false,
                characters: Vec::new(),
                access: AccessLevel::Player,
            },
        );
        self
    }

    /// Whether an account already exists — for "seed from config only if absent",
    /// so the store's credential wins over a stale config password.
    pub fn contains(&self, account: impl Into<AccountName>) -> bool {
        self.accounts.contains_key(&account.into().normalized())
    }

    /// Grant an existing account an access level. Ignored if there is no account.
    pub fn with_access(mut self, account: impl Into<AccountName>, access: AccessLevel) -> Self {
        if let Some(entry) = self.accounts.get_mut(&account.into().normalized()) {
            entry.access = access;
        }
        self
    }

    /// Add a character to an existing account. Ignored if there is no account.
    pub fn with_character(
        mut self,
        account: impl Into<AccountName>,
        name: impl Into<CharacterName>,
    ) -> Self {
        if let Some(entry) = self.accounts.get_mut(&account.into().normalized()) {
            entry.characters.push(CharacterEntry { name: name.into() });
        }
        self
    }

    /// Block an existing account. Ignored if there is no account.
    pub fn blocked(mut self, account: impl Into<AccountName>) -> Self {
        if let Some(entry) = self.accounts.get_mut(&account.into().normalized()) {
            entry.blocked = true;
        }
        self
    }
}

impl Accounts for DevAccounts {
    fn verify(
        &self,
        account: impl Into<RawAccountName>,
        password: impl Into<RawPlaintextPassword>,
    ) -> Result<AccountName, DenyReason> {
        let account = account.into();
        let password = password.into();
        // Reject nonsense before touching the store. These are the widths of
        // the wire fields, so anything longer never came from a real client.
        if account.0.is_empty() || account.0.len() > ACCOUNT_NAME_LENGTH {
            return Err(DenyReason::MalformedAccount);
        }
        if password.0.len() > PASSWORD_LENGTH {
            return Err(DenyReason::MalformedPassword);
        }

        let Some(entry) = self.accounts.get(&account.0.to_lowercase()) else {
            return Err(DenyReason::NoAccount);
        };
        if entry.blocked {
            return Err(DenyReason::Blocked);
        }
        // argon2 verify is constant-time over the digest and rejects a credential
        // that is not a valid hash, so an account row with no real password set
        // can never be logged into.
        if !password::verify(&password, &entry.credential) {
            return Err(DenyReason::BadPassword);
        }
        // The raw name checked out: it names a real, unblocked account with the
        // right password, so it is now a validated `AccountName` — echoed back
        // exactly as typed, case included, since only the lookup folds case.
        Ok(AccountName(account.0))
    }

    fn characters(&self, account: impl Into<AccountName>) -> Vec<CharacterEntry> {
        self.accounts
            .get(&account.into().normalized())
            .map(|entry| entry.characters.clone())
            .unwrap_or_default()
    }

    fn access_level(&self, account: impl Into<AccountName>) -> AccessLevel {
        self.accounts
            .get(&account.into().normalized())
            .map_or(AccessLevel::Player, |entry| entry.access)
    }

    fn create_character(
        &mut self,
        account: AccountName,
        name: RawCharacterName,
    ) -> Result<(u32, CharacterName), DenyReason> {
        // The name is trimmed because the client pads its 30-byte field, and a
        // name that is only spaces is not a name. Width is the wire field's.
        let trimmed = name.0.trim();
        if trimmed.is_empty() || name.0.len() > CHARACTER_NAME_LENGTH {
            return Err(DenyReason::BadCharacter);
        }

        let Some(entry) = self.accounts.get_mut(&account.normalized()) else {
            return Err(DenyReason::NoAccount);
        };
        // The 0xA9 list shows exactly five slots, so a sixth character would be
        // created and then be invisible. Refuse it where the client can hear why.
        if entry.characters.len() >= MIN_CHARACTER_SLOTS {
            return Err(DenyReason::TooManyCharacters);
        }
        // Two characters with one name make 0x5D ambiguous — it echoes the name,
        // not the slot — so a duplicate is refused rather than quietly shadowed.
        if entry
            .characters
            .iter()
            .any(|character| character.name.0.eq_ignore_ascii_case(trimmed))
        {
            return Err(DenyReason::BadCharacter);
        }

        let slot = entry.characters.len() as u32;
        let character = CharacterName(trimmed.to_owned());
        entry.characters.push(CharacterEntry {
            name: character.clone(),
        });
        Ok((slot, character))
    }

    fn delete_character(
        &mut self,
        account: impl Into<AccountName>,
        slot: u32,
    ) -> Result<CharacterName, DenyReason> {
        let Some(entry) = self.accounts.get_mut(&account.into().normalized()) else {
            return Err(DenyReason::NoAccount);
        };
        let index = slot as usize;
        if index >= entry.characters.len() {
            return Err(DenyReason::BadCharacter);
        }
        Ok(entry.characters.remove(index).name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> DevAccounts {
        DevAccounts::new()
            .with_account("admin", "hunter2")
            .with_character("admin", "Lord British")
            .with_account("banned", "x")
            .blocked("banned")
    }

    #[test]
    fn accepts_the_right_password() {
        assert_eq!(
            store().verify("admin", "hunter2"),
            Ok(AccountName("admin".to_owned()))
        );
    }

    #[test]
    fn rejects_the_wrong_password() {
        assert_eq!(
            store().verify("admin", "hunter3"),
            Err(DenyReason::BadPassword)
        );
        assert_eq!(store().verify("admin", ""), Err(DenyReason::BadPassword));
    }

    #[test]
    fn rejects_an_unknown_account() {
        assert_eq!(
            store().verify("nobody", "hunter2"),
            Err(DenyReason::NoAccount)
        );
    }

    #[test]
    fn rejects_a_blocked_account_before_checking_the_password() {
        // Order matters: telling a banned account its password was right is a
        // small thing, but there is no reason to.
        assert_eq!(store().verify("banned", "x"), Err(DenyReason::Blocked));
        assert_eq!(store().verify("banned", "wrong"), Err(DenyReason::Blocked));
    }

    #[test]
    fn account_names_are_case_insensitive() {
        // The client does not round-trip case reliably, and no player expects
        // "Admin" and "admin" to be different accounts.
        assert_eq!(
            store().verify("ADMIN", "hunter2"),
            Ok(AccountName("ADMIN".to_owned()))
        );
        assert_eq!(
            store().verify("AdMiN", "hunter2"),
            Ok(AccountName("AdMiN".to_owned()))
        );
    }

    #[test]
    fn passwords_are_case_sensitive() {
        assert_eq!(
            store().verify("admin", "HUNTER2"),
            Err(DenyReason::BadPassword)
        );
    }

    #[test]
    fn rejects_names_that_no_client_could_have_sent() {
        // The wire field is 30 bytes, so anything longer is a forged packet or
        // a bug upstream. Either way it must not reach the store.
        let long = "x".repeat(ACCOUNT_NAME_LENGTH + 1);
        assert_eq!(
            store().verify(long.as_str(), "x"),
            Err(DenyReason::MalformedAccount)
        );
        assert_eq!(store().verify("", "x"), Err(DenyReason::MalformedAccount));

        let long_password = "x".repeat(PASSWORD_LENGTH + 1);
        assert_eq!(
            store().verify("admin", long_password.as_str()),
            Err(DenyReason::MalformedPassword)
        );
    }

    #[test]
    fn characters_come_back_in_order() {
        let store = DevAccounts::new()
            .with_account("a", "p")
            .with_character("a", "First")
            .with_character("a", "Second");
        let characters = store.characters("a");
        assert_eq!(characters.len(), 2);
        assert_eq!(characters[0].name, "First");
        assert_eq!(characters[1].name, "Second");
    }

    #[test]
    fn an_unknown_account_has_no_characters() {
        assert_eq!(store().characters("nobody"), vec![]);
    }

    #[test]
    fn access_defaults_to_player_and_is_grantable() {
        let store = DevAccounts::new()
            .with_account("admin", "p")
            .with_access("admin", AccessLevel::GameMaster)
            .with_account("plain", "p");
        assert_eq!(store.access_level("admin"), AccessLevel::GameMaster);
        assert_eq!(
            store.access_level("ADMIN"),
            AccessLevel::GameMaster,
            "case-insensitive"
        );
        assert_eq!(store.access_level("plain"), AccessLevel::Player);
        assert_eq!(
            store.access_level("nobody"),
            AccessLevel::Player,
            "unknown is a player, not an error"
        );
    }

    #[test]
    fn create_character_fills_the_first_free_slot() {
        let mut store = DevAccounts::new().with_account("a", "p");
        assert_eq!(
            store.create_character("a".into(), "First".into()),
            Ok((0, CharacterName("First".to_owned())))
        );
        assert_eq!(
            store.create_character("a".into(), "Second".into()),
            Ok((1, CharacterName("Second".to_owned())))
        );
        let characters = store.characters("a");
        assert_eq!(characters.len(), 2);
        assert_eq!(characters[0].name, "First");
        assert_eq!(characters[1].name, "Second");
    }

    #[test]
    fn create_character_survives_to_the_next_read() {
        // The dev store keeps it in memory, which is enough for the new
        // character to be in the list when the client reconnects to play it.
        let mut store = DevAccounts::new().with_account("a", "p");
        let _ = store.create_character("a".into(), "Newbie".into());
        assert_eq!(store.characters("a")[0].name, "Newbie");
    }

    #[test]
    fn create_character_refuses_a_sixth_character() {
        let mut store = DevAccounts::new().with_account("a", "p");
        for index in 0..MIN_CHARACTER_SLOTS {
            assert!(store
                .create_character("a".into(), format!("C{index}").into())
                .is_ok());
        }
        assert_eq!(
            store.create_character("a".into(), "TooMany".into()),
            Err(DenyReason::TooManyCharacters)
        );
    }

    #[test]
    fn create_character_refuses_an_empty_or_overlong_name() {
        let mut store = DevAccounts::new().with_account("a", "p");
        assert_eq!(
            store.create_character("a".into(), "   ".into()),
            Err(DenyReason::BadCharacter)
        );
        assert_eq!(
            store.create_character("a".into(), "".into()),
            Err(DenyReason::BadCharacter)
        );
        let long = "x".repeat(CHARACTER_NAME_LENGTH + 1);
        assert_eq!(
            store.create_character("a".into(), long.into()),
            Err(DenyReason::BadCharacter)
        );
    }

    #[test]
    fn create_character_refuses_a_duplicate_name() {
        let mut store = DevAccounts::new().with_account("a", "p");
        assert!(store.create_character("a".into(), "Twin".into()).is_ok());
        assert_eq!(
            store.create_character("a".into(), "twin".into()),
            Err(DenyReason::BadCharacter),
            "case-insensitively, since the client does not preserve case"
        );
    }

    #[test]
    fn create_character_refuses_an_unknown_account() {
        let mut store = DevAccounts::new();
        assert_eq!(
            store.create_character("nobody".into(), "X".into()),
            Err(DenyReason::NoAccount)
        );
    }

    #[test]
    fn delete_character_removes_the_slot_and_shifts_the_rest() {
        let mut store = DevAccounts::new().with_account("a", "p");
        let _ = store.create_character("a".into(), "First".into());
        let _ = store.create_character("a".into(), "Second".into());
        let _ = store.create_character("a".into(), "Third".into());
        assert_eq!(
            store.delete_character("a", 1),
            Ok(CharacterName("Second".to_owned()))
        );
        let names: Vec<_> = store.characters("a").into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["First", "Third"], "later slots shift down");
    }

    #[test]
    fn delete_character_refuses_an_empty_or_out_of_range_slot() {
        let mut store = DevAccounts::new().with_account("a", "p");
        let _ = store.create_character("a".into(), "Only".into());
        assert_eq!(
            store.delete_character("a", 1),
            Err(DenyReason::BadCharacter)
        );
        assert_eq!(
            store.delete_character("nobody", 0),
            Err(DenyReason::NoAccount)
        );
    }

    #[test]
    fn the_stored_credential_is_a_hash_not_the_plaintext() {
        // A shard's account file is a plausible leak; the password must not be
        // recoverable from it.
        let store = DevAccounts::new().with_account("admin", "hunter2");
        assert_eq!(
            store.verify("admin", "hunter2"),
            Ok(AccountName("admin".to_owned()))
        );
        let credential = &store.accounts["admin"].credential;
        assert!(
            !credential.contains("hunter2"),
            "plaintext must not survive"
        );
        assert!(credential.starts_with("$argon2"), "an argon2 PHC hash");
    }

    #[test]
    fn a_credential_loaded_as_a_hash_is_not_re_hashed() {
        // The boot path: an account already carrying a hash is loaded as-is and
        // still verifies. Re-hashing it (treating it as a plaintext) would lock
        // the account out.
        let phc = password::hash(&PlaintextPassword("secret".to_owned()));
        let store = DevAccounts::new().with_credential("returning", &phc);
        assert_eq!(
            store.verify("returning", "secret"),
            Ok(AccountName("returning".to_owned()))
        );
        assert_eq!(
            store.accounts["returning"].credential, phc,
            "loaded verbatim"
        );
    }
}
