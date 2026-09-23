//! Where the player is: a level, inside the numbering of one world.
//!
//! `currentId` alone is not a place. Each world -- the adventure, and every
//! parallel dimension -- numbers its levels on its own, and the same number
//! names two unrelated levels in two worlds. Entering the dimension of level
//! 6 reads 34 in `xml_deepnight`; the one of level 15 reads 42. So a level
//! number never travels without the world it belongs to.
//!
//! Players write the dimension of level 15 `15.0`. That name is theirs, and
//! nothing in memory carries it: it is not modelled here.

/// A set of levels, as `GameMechanics.setName` names it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum World {
    /// `xml_adventure`: the main world, where a run starts and ends.
    #[default]
    Adventure,
    /// `xml_deepnight`: the parallel dimensions of the adventure.
    Deepnight,
    Hiko,
    Ayame,
    Hk,
}

impl World {
    /// The world a `setName` names, or `None` for a name we do not know.
    pub fn from_set_name(name: &str) -> Option<Self> {
        match name {
            "xml_adventure" => Some(Self::Adventure),
            "xml_deepnight" => Some(Self::Deepnight),
            "xml_hiko" => Some(Self::Hiko),
            "xml_ayame" => Some(Self::Ayame),
            "xml_hk" => Some(Self::Hk),
            _ => None,
        }
    }
}

/// One level: a world, and `currentId` inside it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct Level {
    pub world: World,
    pub id: i64,
}

impl Level {
    pub const fn new(world: World, id: i64) -> Self {
        Self { world, id }
    }
}
