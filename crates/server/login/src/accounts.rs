//! Who is allowed in.

use std::collections::HashMap;

use openshard_protocol::access::AccessLevel;
use openshard_protocol::identity::{AccountName, PlaintextPassword, RawAccountName, RawPlaintextPassword};
use openshard_protocol::login::{ACCOUNT_NAME_LENGTH, DenyReason, PASSWORD_LENGTH};

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
/// # Parameters borrow the typed newtype
///
/// Not `&str`: an account name, a character name and a password are three
/// different things, and threading bare strings through this trait is exactly
/// how a caller ends up passing one where another belongs. Taking `&AccountName`
/// etc. keeps that impossible — a caller (test fixtures included) names the
/// type explicitly at the call site instead of leaning on an implicit `Into`.
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
        account: &RawAccountName,
        password: &RawPlaintextPassword,
    ) -> Result<AccountName, DenyReason>;

    /// The authority the account's characters play with — what staff commands
    /// they may run. Defaults to [`AccessLevel::Player`] so a store that has no
    /// notion of staff grants none, which is the safe direction to be wrong in.
    /// An unknown account is a player, not an error: this is asked after login,
    /// about an account already verified, and the answer only ever *withholds*
    /// authority.
    fn access_level(&self, _account: &AccountName) -> AccessLevel {
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
    pub fn with_account(self, account: &AccountName, password: &PlaintextPassword) -> Self {
        let credential = password::hash(password);
        self.with_credential(account, &credential)
    }

    /// Add an account with an already-hashed credential and no characters.
    ///
    /// The path a stored account takes at boot: its PHC hash is loaded as-is,
    /// never re-hashed. A blank credential (which verifies against nothing)
    /// stands for an account row with no password set.
    pub fn with_credential(mut self, account: &AccountName, credential: &str) -> Self {
        self.accounts.insert(
            account.normalized(),
            DevAccount {
                credential: credential.to_owned(),
                blocked: false,
                access: AccessLevel::Player,
            },
        );
        self
    }

    /// Whether an account already exists — for "seed from config only if absent",
    /// so the store's credential wins over a stale config password.
    pub fn contains(&self, account: &AccountName) -> bool {
        self.accounts.contains_key(&account.normalized())
    }

    /// Grant an existing account an access level. Ignored if there is no account.
    pub fn with_access(mut self, account: &AccountName, access: AccessLevel) -> Self {
        if let Some(entry) = self.accounts.get_mut(&account.normalized()) {
            entry.access = access;
        }
        self
    }

    /// Block an existing account. Ignored if there is no account.
    pub fn blocked(mut self, account: &AccountName) -> Self {
        if let Some(entry) = self.accounts.get_mut(&account.normalized()) {
            entry.blocked = true;
        }
        self
    }
}

impl Accounts for DevAccounts {
    fn verify(
        &self,
        account: &RawAccountName,
        password: &RawPlaintextPassword,
    ) -> Result<AccountName, DenyReason> {
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
        if !password::verify(password, &entry.credential) {
            return Err(DenyReason::BadPassword);
        }
        // The raw name checked out: it names a real, unblocked account with the
        // right password, so it is now a validated `AccountName` — echoed back
        // exactly as typed, case included, since only the lookup folds case.
        Ok(AccountName(account.0.clone()))
    }

    fn access_level(&self, account: &AccountName) -> AccessLevel {
        self.accounts
            .get(&account.normalized())
            .map_or(AccessLevel::Player, |entry| entry.access)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> DevAccounts {
        DevAccounts::new()
            .with_account(&AccountName::new("admin"), &PlaintextPassword::new("hunter2"))
            .with_account(&AccountName::new("banned"), &PlaintextPassword::new("x"))
            .blocked(&AccountName::new("banned"))
    }

    #[test]
    fn accepts_the_right_password() {
        assert_eq!(
            store().verify(
                &RawAccountName::new("admin"),
                &RawPlaintextPassword::new("hunter2")
            ),
            Ok(AccountName::new("admin"))
        );
    }

    #[test]
    fn rejects_the_wrong_password() {
        assert_eq!(
            store().verify(
                &RawAccountName::new("admin"),
                &RawPlaintextPassword::new("hunter3")
            ),
            Err(DenyReason::BadPassword)
        );
        assert_eq!(
            store().verify(&RawAccountName::new("admin"), &RawPlaintextPassword::new("")),
            Err(DenyReason::BadPassword)
        );
    }

    #[test]
    fn rejects_an_unknown_account() {
        assert_eq!(
            store().verify(
                &RawAccountName::new("nobody"),
                &RawPlaintextPassword::new("hunter2")
            ),
            Err(DenyReason::NoAccount)
        );
    }

    #[test]
    fn rejects_a_blocked_account_before_checking_the_password() {
        // Order matters: telling a banned account its password was right is a
        // small thing, but there is no reason to.
        assert_eq!(
            store().verify(&RawAccountName::new("banned"), &RawPlaintextPassword::new("x")),
            Err(DenyReason::Blocked)
        );
        assert_eq!(
            store().verify(
                &RawAccountName::new("banned"),
                &RawPlaintextPassword::new("wrong")
            ),
            Err(DenyReason::Blocked)
        );
    }

    #[test]
    fn account_names_are_case_insensitive() {
        // The client does not round-trip case reliably, and no player expects
        // "Admin" and "admin" to be different accounts.
        assert_eq!(
            store().verify(
                &RawAccountName::new("ADMIN"),
                &RawPlaintextPassword::new("hunter2")
            ),
            Ok(AccountName::new("ADMIN"))
        );
        assert_eq!(
            store().verify(
                &RawAccountName::new("AdMiN"),
                &RawPlaintextPassword::new("hunter2")
            ),
            Ok(AccountName::new("AdMiN"))
        );
    }

    #[test]
    fn passwords_are_case_sensitive() {
        assert_eq!(
            store().verify(
                &RawAccountName::new("admin"),
                &RawPlaintextPassword::new("HUNTER2")
            ),
            Err(DenyReason::BadPassword)
        );
    }

    #[test]
    fn rejects_names_that_no_client_could_have_sent() {
        // The wire field is 30 bytes, so anything longer is a forged packet or
        // a bug upstream. Either way it must not reach the store.
        let long = "x".repeat(ACCOUNT_NAME_LENGTH + 1);
        assert_eq!(
            store().verify(&RawAccountName::new(&long), &RawPlaintextPassword::new("x")),
            Err(DenyReason::MalformedAccount)
        );
        assert_eq!(
            store().verify(&RawAccountName::new(""), &RawPlaintextPassword::new("x")),
            Err(DenyReason::MalformedAccount)
        );

        let long_password = "x".repeat(PASSWORD_LENGTH + 1);
        assert_eq!(
            store().verify(
                &RawAccountName::new("admin"),
                &RawPlaintextPassword::new(&long_password)
            ),
            Err(DenyReason::MalformedPassword)
        );
    }

    #[test]
    fn access_defaults_to_player_and_is_grantable() {
        let store = DevAccounts::new()
            .with_account(&AccountName::new("admin"), &PlaintextPassword::new("p"))
            .with_access(&AccountName::new("admin"), AccessLevel::GameMaster)
            .with_account(&AccountName::new("plain"), &PlaintextPassword::new("p"));
        assert_eq!(
            store.access_level(&AccountName::new("admin")),
            AccessLevel::GameMaster
        );
        assert_eq!(
            store.access_level(&AccountName::new("ADMIN")),
            AccessLevel::GameMaster,
            "case-insensitive"
        );
        assert_eq!(
            store.access_level(&AccountName::new("plain")),
            AccessLevel::Player
        );
        assert_eq!(
            store.access_level(&AccountName::new("nobody")),
            AccessLevel::Player,
            "unknown is a player, not an error"
        );
    }

    #[test]
    fn the_stored_credential_is_a_hash_not_the_plaintext() {
        // A shard's account file is a plausible leak; the password must not be
        // recoverable from it.
        let store =
            DevAccounts::new().with_account(&AccountName::new("admin"), &PlaintextPassword::new("hunter2"));
        assert_eq!(
            store.verify(
                &RawAccountName::new("admin"),
                &RawPlaintextPassword::new("hunter2")
            ),
            Ok(AccountName::new("admin"))
        );
        let credential = &store.accounts["admin"].credential;
        assert!(!credential.contains("hunter2"), "plaintext must not survive");
        assert!(credential.starts_with("$argon2"), "an argon2 PHC hash");
    }

    #[test]
    fn a_credential_loaded_as_a_hash_is_not_re_hashed() {
        // The boot path: an account already carrying a hash is loaded as-is and
        // still verifies. Re-hashing it (treating it as a plaintext) would lock
        // the account out.
        let phc = password::hash(&PlaintextPassword::new("secret"));
        let store = DevAccounts::new().with_credential(&AccountName::new("returning"), &phc);
        assert_eq!(
            store.verify(
                &RawAccountName::new("returning"),
                &RawPlaintextPassword::new("secret")
            ),
            Ok(AccountName::new("returning"))
        );
        assert_eq!(store.accounts["returning"].credential, phc, "loaded verbatim");
    }
}
