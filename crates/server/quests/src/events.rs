use openshard_protocol::serial::Serial;

/// Which objective a quest definition's `objectives` names — the same index a
/// `QuestState::progress` slot is at. Crosses the event bus into scripting
/// (Community Pack content reads it), so a bare `usize` here would be a plain
/// integer at the one boundary where a pack author has nothing else to check
/// it against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ObjectiveIndex(pub usize);

/// A player accepted a quest.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestAccepted {
    /// Who took it.
    pub player: Serial,
    /// Which quest, by the pack's key.
    pub key: String,
}

/// A player turned an offered quest down. Nothing was started.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestRefused {
    /// Who refused.
    pub player: Serial,
    /// Which quest, by the pack's key.
    pub key: String,
}

/// A player gave up on a quest they had taken.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestResigned {
    /// Who resigned.
    pub player: Serial,
    /// Which quest, by the pack's key.
    pub key: String,
}

/// An objective moved — a kill counted, an item found, a leg of a journey walked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestObjectiveUpdated {
    /// Whose quest.
    pub player: Serial,
    /// Which quest, by the pack's key.
    pub key: String,
    /// Which objective, by its index in the definition.
    pub objective: ObjectiveIndex,
    /// How far it has got now.
    pub progress: u16,
    /// How far it needs to get.
    pub goal: u16,
}

/// A timed quest ran out of time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestFailed {
    /// Whose quest.
    pub player: Serial,
    /// Which quest, by the pack's key.
    pub key: String,
}

/// A quest was turned in and paid.
///
/// The pack's hook for anything the core's flat reward list cannot express — a
/// title, a skill, a follow-up quest, a line of dialogue. The core has already
/// paid the declared rewards by the time this is read; a script *adds*, exactly
/// as it does off `CorpseCreated`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestCompleted {
    /// Who finished it.
    pub player: Serial,
    /// Which quest, by the pack's key.
    pub key: String,
    /// Who it was turned in to, if the giver is still around.
    pub giver: Option<Serial>,
}
