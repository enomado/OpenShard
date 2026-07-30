//! Who is allowed in.

use std::collections::HashMap;
use std::fmt;

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
    /// Everything about a login that is a *lookup*: whether the account exists,
    /// whether it is blocked, and whether the name and password are even shapes
    /// a client could have sent. On success it hands back the credential the
    /// offered password still has to match.
    ///
    /// Takes the *raw*, not-yet-validated wire types — see
    /// `openshard_protocol::identity`'s module docs — and the account name it
    /// resolves rides in the returned [`Credential`]: the only way to a
    /// validated [`AccountName`] is through this check (or trusted config
    /// seeding via [`DevAccounts::with_account`]).
    ///
    /// The password comes in for its *length* alone. A field wider than the
    /// wire's is a forged packet rather than a wrong password, and it is a
    /// different refusal; nothing here compares it. That comparison is
    /// [`CredentialCheck::run`], and it is split off because it is thousands of
    /// times more expensive than everything above — see [`Credential`].
    ///
    /// Returns the reason on failure so the caller can log it. What the client
    /// is told is a separate decision — see [`DenyReason::wire_code`].
    fn credential(
        &self,
        account: &RawAccountName,
        password: &RawPlaintextPassword,
    ) -> Result<Credential, DenyReason>;

    /// The whole check, hash comparison included, on the caller's own thread.
    ///
    /// For a caller with nothing to stall: tests, tools, a fixture. The shard
    /// deliberately does not use it — [`LoginServer::handle`] hands the slow half
    /// back so the loop can run it somewhere the tick is not waiting on it. See
    /// [`Credential`].
    ///
    /// [`LoginServer::handle`]: crate::LoginServer::handle
    fn verify(
        &self,
        account: &RawAccountName,
        password: &RawPlaintextPassword,
    ) -> Result<AccountName, DenyReason> {
        let (account, check) = self.credential(account, password)?.against(password.clone());
        match check.run() {
            PasswordVerdict::Matched => Ok(account),
            PasswordVerdict::Rejected => Err(DenyReason::BadPassword),
        }
    }

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

/// What a login still has to prove, once the store has had its say: which
/// account the raw name turned out to name, and the hash the offered password
/// must match.
///
/// # Why a login is two halves
///
/// They cost wildly different amounts. Everything a store knows — does this
/// account exist, is it blocked, is the name a shape a client could have sent —
/// is a map lookup and two length checks. Comparing the password is argon2:
/// 19 MiB of memory and two passes, tens of milliseconds, deliberately, because
/// that is what makes a stolen credential file expensive to crack.
///
/// Tens of milliseconds is most of a 50 ms tick. Run on the shard's loop, one
/// login stalls the simulation for every player on the shard; a handful at once
/// stalls it visibly. So the cheap half stays where the accounts are and the
/// expensive half becomes a value the caller can carry to a thread of its own —
/// see [`CredentialCheck`], and `docs/connection_state.md` S6.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential {
    account: AccountName,
    phc: String,
}

impl Credential {
    /// The credential of `account`: an argon2 PHC string, as
    /// [`DevAccount::credential`] holds it. For implementors of [`Accounts`] —
    /// everybody else receives one.
    pub fn new(account: AccountName, phc: &str) -> Self {
        Self {
            account,
            phc: phc.to_owned(),
        }
    }

    /// Split into the identity the login keeps and the work it hands out.
    ///
    /// The account deliberately does *not* travel with the check. What comes back
    /// from a password comparison is yes-or-no about a credential; *who* is being
    /// logged in stays with the state machine that will act on the answer. A
    /// state machine with facts kept outside it is a state machine that can
    /// disagree with itself — and an identity that travelled with the work could
    /// come back attached to a different connection's verdict.
    pub fn against(self, offered: RawPlaintextPassword) -> (AccountName, CredentialCheck) {
        (
            self.account,
            CredentialCheck {
                phc: self.phc,
                offered,
            },
        )
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The hash is redacted for the same reason the plaintext is: a PHC string
        // in a log file is an offline cracking target that escaped the store.
        write!(f, "Credential({:?}, <redacted>)", self.account)
    }
}

/// One password comparison, owned and ready to run anywhere: a blocking task, a
/// thread, or the caller's own stack in a test.
///
/// It carries no identity — see [`Credential::against`] — so the only thing it
/// can answer is [`PasswordVerdict`].
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialCheck {
    phc: String,
    offered: RawPlaintextPassword,
}

impl CredentialCheck {
    /// Run the comparison. Slow by design, with the parameters the stored hash
    /// carries: this is the call that must not happen on the shard loop.
    #[must_use]
    pub fn run(self) -> PasswordVerdict {
        if password::verify(&self.offered, &self.phc) {
            PasswordVerdict::Matched
        } else {
            PasswordVerdict::Rejected
        }
    }
}

impl fmt::Debug for CredentialCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialCheck(<redacted>)")
    }
}

/// Whether an offered password matched the credential it was checked against.
///
/// Not a `bool`: it crosses a channel in the shard, beside a connection id, and
/// arrives at a state machine that will let somebody in on it. A two-variant
/// enum says which way round it is at the call site as well as the definition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PasswordVerdict {
    /// The password is the one the credential was made from.
    Matched,
    /// It is not — or the credential is not a hash at all, which verifies
    /// against nothing.
    Rejected,
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
    fn credential(
        &self,
        account: &RawAccountName,
        password: &RawPlaintextPassword,
    ) -> Result<Credential, DenyReason> {
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
        // The raw name named a real, unblocked account, so it is a validated
        // `AccountName` as far as the store is concerned — echoed back exactly as
        // typed, case included, since only the lookup folds case. What it is not
        // yet is *authenticated*: nothing here has looked at the password, and the
        // name does not leave this crate until the check below has run.
        //
        // The credential is handed over as it is stored. A row with no real
        // password set holds something that is not a hash at all, and argon2
        // verify rejects it — so "no password" can never be logged into, without
        // a special case here.
        Ok(Credential::new(AccountName(account.0.clone()), &entry.credential))
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
    fn the_lookup_stops_short_of_the_password() {
        // The split this exists for. Everything a store knows is decided here;
        // whether the password is right is not, and the wrong one gets exactly as
        // far as the right one. That is what lets the shard run the rest of it
        // somewhere its tick is not waiting — see `Credential`.
        let store = store();
        for password in ["hunter2", "wrong", ""] {
            assert!(
                store
                    .credential(
                        &RawAccountName::new("admin"),
                        &RawPlaintextPassword::new(password)
                    )
                    .is_ok(),
                "{password:?} should reach the check, right or wrong"
            );
        }
        // And the refusals that do belong here still happen here, before any
        // argon2 runs: an unknown account, a blocked one, a forged field.
        assert_eq!(
            store.credential(&RawAccountName::new("banned"), &RawPlaintextPassword::new("x")),
            Err(DenyReason::Blocked)
        );
    }

    #[test]
    fn the_check_is_what_decides() {
        // The other half, run on this thread the way the shard runs it on a
        // blocking task. The account comes off the *lookup* and never travels
        // with the check, so all the check can say is yes or no.
        let store = store();
        let credential = store
            .credential(
                &RawAccountName::new("admin"),
                &RawPlaintextPassword::new("hunter2"),
            )
            .expect("admin exists");
        let (account, check) = credential.against(RawPlaintextPassword::new("hunter2"));
        assert_eq!(account, AccountName::new("admin"));
        assert_eq!(check.run(), PasswordVerdict::Matched);

        let (_, check) = store
            .credential(&RawAccountName::new("admin"), &RawPlaintextPassword::new("wrong"))
            .expect("the lookup does not care")
            .against(RawPlaintextPassword::new("wrong"));
        assert_eq!(check.run(), PasswordVerdict::Rejected);
    }

    #[test]
    fn neither_half_of_a_login_prints_a_secret() {
        // Both of these are logged by the shard on the paths that carry them —
        // `?state` in the "out of order" warning reaches the whole login session.
        // A password in a log line is the leak this crate exists to prevent, and
        // a PHC hash is one an attacker can take home and grind on.
        let credential = store()
            .credential(
                &RawAccountName::new("admin"),
                &RawPlaintextPassword::new("hunter2"),
            )
            .expect("admin exists");
        let printed = format!("{credential:?}");
        assert!(
            !printed.contains("$argon2"),
            "the hash must not survive: {printed}"
        );
        let (_, check) = credential.against(RawPlaintextPassword::new("hunter2"));
        let printed = format!("{check:?}");
        assert!(!printed.contains("hunter2"), "the plaintext must not survive");
        assert!(!printed.contains("$argon2"), "nor the hash");
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
