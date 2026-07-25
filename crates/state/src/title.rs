//! What a character is *called* — ServUO's fame/karma title table.
//!
//! Data and a pure function over it, so it sits here with [`creature_name`] and the
//! body table rather than in `combat`: two crates read a title (the single-click label
//! and the `0xD6` tooltip) and only one awards the standing behind it. The *rules* —
//! how fame and karma are earned and the curve that diminishes them — are
//! `combat::titles`.

use crate::components::{Body, Fame, Karma};
use crate::WorldState;
use openshard_entities::EntityId;

/// One band of the title table: a fame ceiling and the karma bands inside it.
struct FameBand {
    /// The highest fame this band covers.
    fame: i32,
    /// `(karma ceiling, title)`, in ascending order of karma. `{}` is the name and
    /// `{lord}` the Lord/Lady the top fame band earns.
    karma: &'static [(i32, &'static str)],
}

/// ServUO's `m_FameEntries`, verbatim. Five fame bands of eleven karma bands each.
const TITLES: &[FameBand] = &[
    FameBand {
        fame: 1249,
        karma: &[
            (-10000, "The Outcast {}"),
            (-5000, "The Despicable {}"),
            (-2500, "The Scoundrel {}"),
            (-1250, "The Unsavory {}"),
            (-625, "The Rude {}"),
            (624, "{}"),
            (1249, "The Fair {}"),
            (2499, "The Kind {}"),
            (4999, "The Good {}"),
            (9999, "The Honest {}"),
            (10000, "The Trustworthy {}"),
        ],
    },
    FameBand {
        fame: 2499,
        karma: &[
            (-10000, "The Wretched {}"),
            (-5000, "The Dastardly {}"),
            (-2500, "The Malicious {}"),
            (-1250, "The Dishonorable {}"),
            (-625, "The Disreputable {}"),
            (624, "The Notable {}"),
            (1249, "The Upstanding {}"),
            (2499, "The Respectable {}"),
            (4999, "The Honorable {}"),
            (9999, "The Commendable {}"),
            (10000, "The Estimable {}"),
        ],
    },
    FameBand {
        fame: 4999,
        karma: &[
            (-10000, "The Nefarious {}"),
            (-5000, "The Wicked {}"),
            (-2500, "The Vile {}"),
            (-1250, "The Ignoble {}"),
            (-625, "The Notorious {}"),
            (624, "The Prominent {}"),
            (1249, "The Reputable {}"),
            (2499, "The Proper {}"),
            (4999, "The Admirable {}"),
            (9999, "The Famed {}"),
            (10000, "The Great {}"),
        ],
    },
    FameBand {
        fame: 9999,
        karma: &[
            (-10000, "The Dread {}"),
            (-5000, "The Evil {}"),
            (-2500, "The Villainous {}"),
            (-1250, "The Sinister {}"),
            (-625, "The Infamous {}"),
            (624, "The Renowned {}"),
            (1249, "The Distinguished {}"),
            (2499, "The Eminent {}"),
            (4999, "The Noble {}"),
            (9999, "The Illustrious {}"),
            (10000, "The Glorious {}"),
        ],
    },
    FameBand {
        fame: 10000,
        karma: &[
            (-10000, "The Dread {lord} {}"),
            (-5000, "The Evil {lord} {}"),
            (-2500, "The Dark {lord} {}"),
            (-1250, "The Sinister {lord} {}"),
            (-625, "The Dishonored {lord} {}"),
            (624, "{lord} {}"),
            (1249, "The Distinguished {lord} {}"),
            (2499, "The Eminent {lord} {}"),
            (4999, "The Noble {lord} {}"),
            (9999, "The Illustrious {lord} {}"),
            (10000, "The Glorious {lord} {}"),
        ],
    },
];

/// The name a mobile is known by, title and all — ServUO's `Titles.ComputeFameTitle`.
///
/// The band is the first whose fame ceiling the mobile reaches (or the last), then the
/// first karma ceiling inside it. `female` picks Lady over Lord, which only the top
/// fame band uses.
#[must_use]
pub fn compute_title(name: &str, fame: i32, karma: i32, female: bool) -> String {
    let band = TITLES
        .iter()
        .find(|band| fame <= band.fame)
        .unwrap_or(&TITLES[TITLES.len() - 1]);
    let pattern = band
        .karma
        .iter()
        .find(|&&(ceiling, _)| karma <= ceiling)
        .map_or(band.karma[band.karma.len() - 1].1, |&(_, title)| title);
    pattern
        .replace("{lord}", if female { "Lady" } else { "Lord" })
        .replace("{}", name)
}

/// A mobile's earned name, or its plain one when it has earned nothing.
///
/// ServUO shows a fame title to the mobile itself always and to onlookers only once its
/// fame reaches 5000 (`ShowFameTitle`); below that a stranger reads the bare name. This
/// is the onlooker's view, which is the one every label in the engine draws.
#[must_use]
pub fn titled_name(state: &WorldState, mobile: EntityId, name: &str) -> String {
    let fame = state.registry.get::<Fame>(mobile).map_or(0, |f| f.0);
    if fame < 5000 {
        return name.to_owned();
    }
    let karma = state.registry.get::<Karma>(mobile).map_or(0, |k| k.0);
    let female = state
        .registry
        .get::<Body>(mobile)
        .is_some_and(|body| body.id == 0x0191 || body.id == 0x0193);
    compute_title(name, fame, karma, female)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_titles_are_servuos() {
        assert_eq!(compute_title("Rowena", 0, 0, false), "Rowena");
        assert_eq!(
            compute_title("Rowena", 1000, 5000, false),
            "The Honest Rowena"
        );
        assert_eq!(
            compute_title("Rowena", 1000, -6000, false),
            "The Despicable Rowena"
        );
        assert_eq!(
            compute_title("Rowena", 9000, 20000, false),
            "The Glorious Rowena"
        );
    }

    #[test]
    fn only_the_top_band_earns_a_lordship() {
        // `{1}` in ServUO's table, and it appears in exactly one fame band.
        assert_eq!(compute_title("Rowena", 20000, 0, true), "Lady Rowena");
        assert_eq!(compute_title("Rowena", 20000, 0, false), "Lord Rowena");
        assert_eq!(
            compute_title("Rowena", 20000, 20000, true),
            "The Glorious Lady Rowena"
        );
        for karma in [-20000, 0, 20000] {
            assert!(
                !compute_title("Rowena", 9999, karma, true).contains("Lady"),
                "karma {karma}"
            );
        }
    }

    #[test]
    fn a_placeholder_never_survives_into_a_title() {
        for fame in [0, 1249, 2500, 5000, 10000, 32000] {
            for karma in [-32000, -1000, 0, 1000, 32000] {
                let title = compute_title("Rowena", fame, karma, false);
                assert!(!title.contains('{'), "{fame}/{karma}: {title}");
                assert!(title.contains("Rowena"), "{fame}/{karma}: {title}");
            }
        }
    }
}
