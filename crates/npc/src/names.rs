//! A townsperson's name: a personal name and the title of its trade.
//!
//! # Two halves, from two places
//!
//! ServUO builds a vendor's label out of a `Name` drawn from `Data/names.xml`
//! (`NameList.RandomName("male")`) and a `Title` its class fixes — "the blacksmith",
//! "the banker" — and the client draws the two together. That split is why a town
//! reads as a town: everybody has a trade *and* a name.
//!
//! What the shard had instead was one string, and the pack was sending only the
//! title, so all thirty-eight bankers in Felucca were called "the banker". The
//! title is the pack's (it knows the profession); the personal name is the core's,
//! generated off the world's seeded [`Rng`] so a shard names the same town twice.
//!
//! # Why the list here is a default and not the whole of ServUO's
//!
//! `Data/names.xml` holds 1,500 male and 2,132 female names. Those belong to the
//! operator's own ServUO checkout, not in this repository — the same reason no
//! client files are here. So the core carries a spread of them wide enough that a
//! full Felucca does not read as repetitive, and a pack that wants the whole list
//! registers it (see [`crate::speech`]) and overrides this one.

use openshard_state::rng::Rng;

const MALE_NAMES: &[&str] = &[
    "Aaron",
    "Achilles",
    "Adolph",
    "Aimery",
    "Alaric",
    "Aleron",
    "Alonzo",
    "Ammon",
    "Anker",
    "Aren",
    "Arlo",
    "Arsenio",
    "Audun",
    "Balbo",
    "Baran",
    "Barry",
    "Bayani",
    "Belen",
    "Benton",
    "Bevan",
    "Blorn",
    "Bowie",
    "Bran",
    "Brendon",
    "Brinley",
    "Bryan",
    "Burr",
    "Caleb",
    "Carlo",
    "Casey",
    "Chad",
    "Chapman",
    "Chike",
    "Claude",
    "Colby",
    "Corbett",
    "Creighton",
    "Dag",
    "Damion",
    "Darrel",
    "Delano",
    "Denver",
    "Dillon",
    "Donovan",
    "Duncan",
    "Dymas",
    "Edwin",
    "Elkan",
    "Emil",
    "Erol",
    "Everett",
    "Fenton",
    "Fitzgerald",
    "Franek",
    "Fulton",
    "Garner",
    "Gavrie",
    "Gideon",
    "Grady",
    "Griffen",
    "Hadley",
    "Hans",
    "Harvey",
    "Herbert",
    "Horton",
    "Iain",
    "Itzak",
    "Jagger",
    "Jarrod",
    "Jeffrey",
    "Jin",
    "Jorgen",
    "Kadin",
    "Kardal",
    "Keelan",
    "Ken",
    "Keona",
    "Kiefer",
    "Kliftin",
    "Lamar",
    "Lear",
    "Leonard",
    "Lincoln",
    "Lucian",
    "Mackenzie",
    "Marcos",
    "Marshal",
    "Maurice",
    "Meyer",
    "Milton",
    "Motega",
    "Neal",
    "Nicholas",
    "Norton",
    "Olaf",
    "Oscar",
    "Pascal",
    "Peder",
    "Pierce",
    "Quillan",
    "Raleigh",
    "Rankin",
    "Redmond",
    "Reuben",
    "Ridgley",
    "Rockwell",
    "Ronan",
    "Rudd",
    "Ryder",
    "Sandon",
    "Sean",
    "Seward",
    "Shing",
    "Solomon",
    "Stephan",
    "Sulaiman",
    "Tajo",
    "Tem",
    "Theodore",
    "Toby",
    "Tremaine",
    "Tymon",
    "Vinson",
    "Wallace",
    "Webster",
    "Wilson",
    "Yancey",
    "Zaid",
];

const FEMALE_NAMES: &[&str] = &[
    "Aba",
    "Achen",
    "Adonia",
    "Aiko",
    "Ajinora",
    "Alaqua",
    "Alexandrina",
    "Allinora",
    "Amadi",
    "Ambis",
    "Andi",
    "Anieli",
    "Anteia",
    "Arabella",
    "Aricia",
    "Ashleigh",
    "Athena",
    "Aurora",
    "Aydee",
    "Azora",
    "Basha",
    "Becky",
    "Bess",
    "Birdie",
    "Braina",
    "Briony",
    "Calida",
    "Candide",
    "Carling",
    "Casilda",
    "Celandine",
    "Charissa",
    "Chika",
    "Cicely",
    "Colette",
    "Cortney",
    "Dagmar",
    "Daphene",
    "Daya",
    "Delicia",
    "Diamanta",
    "Dominique",
    "Dulcinea",
    "Edwina",
    "Elizabeeth",
    "Emelie",
    "Esperanza",
    "Evadine",
    "Farima",
    "Finola",
    "Fuscienne",
    "Geogia",
    "Gilen",
    "Gracie",
    "Haimi",
    "Haya",
    "Hilary",
    "Imogene",
    "Isabel",
    "Jael",
    "Jannelle",
    "Jennettia",
    "Jobey",
    "Jordane",
    "Jun",
    "Kala",
    "Kambo",
    "Karida",
    "Kate",
    "Keelin",
    "Kenyangi",
    "Kimmy",
    "Koressa",
    "Lane",
    "Lavern",
    "Lenor",
    "Lien",
    "Liv",
    "Lucretia",
    "Lysel",
    "Magdaline",
    "Malka",
    "Maren",
    "Mariel",
    "Marsha",
    "Maureen",
    "Melanie",
    "Mesha",
    "Mindel",
    "Mitzi",
    "Nadine",
    "Narda",
    "Neoma",
    "Niobe",
    "Noreen",
    "Okelani",
    "Orianna",
    "Pascale",
    "Peninna",
    "Pila",
    "Rachel",
    "Raven",
    "Rhiamon",
    "Roanna",
    "Rosemary",
    "Sabra",
    "Sally",
    "Sarisha",
    "Senta",
    "Shantha",
    "Shina",
    "Simba",
    "Solita",
    "Stesha",
    "Sylvia",
    "Takoda",
    "Tao",
    "Teryn",
    "Thirza",
    "Tora",
    "Trisha",
    "Valentina",
    "Verity",
    "Wanda",
    "Xanthe",
    "Yeva",
    "Zaltana",
    "Ziazan",
];

/// A personal name for a townsperson, from the core's default lists.
///
/// The gender picks the list, as `BaseVendor.InitBody` does. A pack that
/// registered its own names is served by [`crate::speech::registered_name`]
/// instead; this is the fallback a bare shard runs on.
#[must_use]
pub fn personal_name(rng: &mut Rng, female: bool) -> &'static str {
    let list = if female { FEMALE_NAMES } else { MALE_NAMES };
    list[rng.below(list.len() as u32) as usize]
}

/// A townsperson's full label: a personal name and its trade, e.g.
/// "Rowena the blacksmith".
///
/// `title` is what the pack sends — already in ServUO's form, with the leading
/// "the". An empty title gives the bare name, so a nameless-trade NPC still reads.
#[must_use]
pub fn townsperson_name(rng: &mut Rng, title: &str, female: bool) -> String {
    let name = personal_name(rng, female);
    let title = title.trim();
    if title.is_empty() {
        name.to_owned()
    } else {
        format!("{name} {title}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_full_name_carries_the_trade_after_the_person() {
        // The order matters: the client draws the label as one string, and
        // "the blacksmith Rowena" is not how UO reads.
        let mut rng = Rng::new(0x51ED);
        let name = townsperson_name(&mut rng, "the blacksmith", false);
        assert!(name.ends_with(" the blacksmith"), "{name}");
        assert!(!name.starts_with("the "), "{name}");
    }

    #[test]
    fn the_same_seed_names_the_same_townsperson() {
        let mut a = Rng::new(9);
        let mut b = Rng::new(9);
        assert_eq!(
            townsperson_name(&mut a, "the banker", true),
            townsperson_name(&mut b, "the banker", true)
        );
    }

    #[test]
    fn a_tradeless_npc_keeps_a_bare_name() {
        let mut rng = Rng::new(3);
        let name = townsperson_name(&mut rng, "  ", false);
        assert!(!name.contains(' '), "{name}");
        assert!(!name.is_empty());
    }

    #[test]
    fn the_lists_are_wide_enough_for_a_whole_facet() {
        // 738 townsfolk are placed at once. A list of twenty — which is what this
        // replaced — puts the same six names on every street in Britain.
        assert!(MALE_NAMES.len() >= 100, "{}", MALE_NAMES.len());
        assert!(FEMALE_NAMES.len() >= 100, "{}", FEMALE_NAMES.len());
        assert_eq!(
            MALE_NAMES.len(),
            MALE_NAMES.iter().collect::<HashSet<_>>().len(),
            "a duplicate name wastes a slot in the roll"
        );
        assert_eq!(
            FEMALE_NAMES.len(),
            FEMALE_NAMES.iter().collect::<HashSet<_>>().len()
        );
        assert!(
            MALE_NAMES
                .iter()
                .chain(FEMALE_NAMES)
                .all(|n| !n.is_empty() && !n.contains(' ')),
            "a personal name is one word, or the title runs into it"
        );
    }

    #[test]
    fn the_two_lists_do_not_name_the_same_person() {
        // Not a correctness rule, a variety one: an overlap means a hue-and-body
        // female NPC can be called a name every male NPC also uses.
        let men: HashSet<_> = MALE_NAMES.iter().collect();
        let overlap = FEMALE_NAMES.iter().filter(|n| men.contains(n)).count();
        assert_eq!(overlap, 0, "{overlap} names appear on both lists");
    }
}
