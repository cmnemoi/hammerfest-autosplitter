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

    /// The `setName` of this world, as the game writes it.
    pub const fn set_name(self) -> &'static str {
        match self {
            Self::Adventure => "xml_adventure",
            Self::Deepnight => "xml_deepnight",
            Self::Hiko => "xml_hiko",
            Self::Ayame => "xml_ayame",
            Self::Hk => "xml_hk",
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

/// A move the splits must follow.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Crossing {
    /// Forward inside one world, by this many levels.
    Forward(i64),
    /// Into another world. The two numbers share no numbering, so the move
    /// has no size.
    OtherWorld,
}

/// The level the player is on, and where they left each world.
#[derive(Clone, Debug, Default)]
pub struct Route {
    here: Option<Level>,
    /// The level each world showed when the player last left it.
    left: [Option<i64>; 5],
}

impl Route {
    /// The player is read on `level`. Is that a crossing?
    ///
    /// Back in a world, the game first shows the level the player left it
    /// from, then the level they arrive on: the adventure reads 6, then 7,
    /// sixty milliseconds apart. That first read is not a place the player
    /// reaches. It is ignored, and the crossing is measured from the world
    /// they come from.
    pub fn enter(&mut self, level: Level) -> Option<Crossing> {
        let Some(here) = self.here else {
            self.here = Some(level);
            return None;
        };
        if level.world != here.world {
            if self.left[level.world as usize] == Some(level.id) {
                return None;
            }
            self.left[here.world as usize] = Some(here.id);
            self.here = Some(level);
            return Some(Crossing::OtherWorld);
        }
        self.here = Some(level);
        (level.id > here.id).then_some(Crossing::Forward(level.id - here.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_shows_the_set_name_it_was_read_from() {
        for name in [
            "xml_adventure",
            "xml_deepnight",
            "xml_hiko",
            "xml_ayame",
            "xml_hk",
        ] {
            let world = World::from_set_name(name).expect(name);
            assert_eq!(world.set_name(), name);
        }
    }
}
