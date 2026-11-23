use crate::logic_items::ItemId;
use crate::slice::data::Data;
use crate::slice::support::Plan;
use crate::world::{LocId, World};

pub struct Spot<'a> {
    pub d: &'a Data,
    pub w: &'a World,
    pub p: &'a Plan,
    pub it: ItemId,
    pub l: LocId,
    pub name: &'a str,
    pub item: &'a str,
}

pub type Rule = fn(&Spot) -> Option<bool>;

/// The rules the settings ask for: they stay when every item may go everywhere.
pub const SETTING_RULES: [(&str, Rule); 7] = [
    ("ganon's boss key on the light arrows cutscene", lacs_key),
    ("dungeon rewards at the end of dungeons", end_of_dungeons),
    ("excluded locations", excluded),
    ("silver rupees", silver),
    ("adult trade items", adult_trade),
    ("skipped locations", skipped),
    ("song spots", songs),
];

pub fn setting_verdict(s: &Spot) -> Option<bool> {
    SETTING_RULES.iter().find_map(|(_, r)| r(s))
}

/// The adult trade chain keeps its items at home: the sequence is not shuffled.
fn adult_trade(s: &Spot) -> Option<bool> {
    let child: [&str; 2] = ["Item_WeirdEgg", "Item_ZeldaLetter"];
    s.d.item(s.item)
        .filter(|i| i.actor.as_deref().is_some_and(|a| crate::flow_extra::trade::chain(a) && !child.contains(&a)))
        .is_some_and(|i| !crate::flow_extra::trade::keeps(s.d, s.name, i))
        .then_some(false)
}

fn lacs_key(s: &Spot) -> Option<bool> {
    if s.p.keys.ganon != crate::slice::support::Keep::Lacs {
        return None;
    }
    if s.name == crate::lacs_key::LOC {
        return Some(s.item == crate::lacs_key::ITEM);
    }
    (s.item == crate::lacs_key::ITEM).then_some(false)
}

/// Dungeon Rewards = End of dungeons: a Stone or Medallion only goes to a boss or Rauru reward
/// event, and those events only take a Stone or Medallion. A fitting pair is left to the other rules.
fn end_of_dungeons(s: &Spot) -> Option<bool> {
    if !s.p.rewards.dungeon_own {
        return None;
    }
    let item: bool = s.d.item(s.item).is_some_and(|i| i.kind == "DungeonReward");
    let spot: bool = s.d.flow.by_name(s.name).is_some_and(|l| l.cat == crate::slice::support::DUNGEON_REWARD_CAT);
    (item != spot).then_some(false)
}

fn excluded(s: &Spot) -> Option<bool> {
    (!s.p.excluded.is_empty() && s.w.items.adv[s.it] && s.p.excluded.iter().any(|e| *e == s.name)).then_some(false)
}

fn silver(s: &Spot) -> Option<bool> {
    crate::slice::silver::at(s.d, s.w, s.p, s.it, s.l)
}

fn skipped(s: &Spot) -> Option<bool> {
    (crate::slice::edit::skipped_locations(s.p).contains(&s.name) && !s.d.item(s.item).is_some_and(|i| i.actor.is_some() || !i.tiers.is_empty())).then_some(false)
}

fn songs(s: &Spot) -> Option<bool> {
    crate::slice::song_spots::gate(s.d, s.p, s.name, s.item)
}

