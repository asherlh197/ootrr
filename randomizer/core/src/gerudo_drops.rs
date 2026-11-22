//! The four native Thieves' Hideout key drops become ordinary shuffled checks when Hideout keys
//! are shuffled. They keep their own logic locations, so the fill chooses one item per drop from
//! the same pool that can sit on the ground, and the seed records what each guard drops.

use std::collections::BTreeMap as BM;

use crate::error::{Error as E, Res};
use crate::settings::seed::SeedFile;
use crate::slice::data::Data;
use crate::slice::key_spots::{self, Spot};
use crate::slice::support::{Keep, Plan};
use crate::world::{LocId, World};

/// Whether the guard drops hold shuffled items instead of their vanilla Hideout keys.
pub fn active(p: &Plan) -> bool {
    p.hideout != Keep::Vanilla
}

/// An item that can be picked up off the floor through a generated pickup pack. Literal dungeon
/// keys and shipped world pickups are refused by `key_spots`, which is the same filter pots use.
pub fn eligible(d: &Data, item: &str) -> bool {
    key_spots::placeable(d, item, Spot::Free)
}

/// Add the four carpenter-drop checks to the shuffled pool. The native locations stay in the logic
/// world in every mode; only Hideout-key shuffle turns them into checks.
pub fn add_checks(w: &World, p: &Plan, free: &mut Vec<LocId>, left: &mut BM<String, usize>) {
    if !active(p) {
        return;
    }
    for name in crate::hideout_keys::NATIVE {
        match w.loc(name) {
            Some(l) => free.push(l),
            None => {
                *left.entry(format!("{name} is not in the logic world")).or_insert(0) += 1;
            }
        }
    }
}

/// The pickup-pack actor each native drop must spawn, keyed by its logic location name.
pub fn actors(d: &Data, seed: &SeedFile, fixed: &BM<String, String>) -> Res<BM<String, String>> {
    let mut out: BM<String, String> = BM::new();
    for name in crate::hideout_keys::NATIVE {
        let item: &str = seed
            .placements
            .get(name)
            .ok_or_else(|| E::Msg(format!("{name} has no item in this seed")))?;
        let pick: key_spots::Pick = key_spots::resolve_at(d, name, item, fixed.get(name).map(|s| s.as_str()))
            .map_err(|e| E::Msg(format!("{name} holds {item}, which cannot drop from a guard: {e}")))?;
        out.insert(name.to_string(), pick.pack);
    }
    Ok(out)
}
