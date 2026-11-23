//! Ring pickups need their own get event to save their pouch identity. The working key-ring build
//! ships one small AINB per ring; this module reuses that shape with our short flag names.
//!
//! The AINB deliberately does *not* write dungeon keys or the applied flag. All credit comes from
//! the pickup flows (world pickups, chests and gifts), which use the same `GameData` actions main
//! randomizer uses. An earlier version also wrote credit in the AINB; its int writes did not take
//! effect on `DungeonInfo.*`, but its applied-flag write did, so chest flows saw the ring as already
//! credited and skipped their working key write.

use crate::ainb::{audit, Ainb};
use crate::error::{Error as E, Res};

const FOREST: &[u8] = include_bytes!("../../../data/ring_events/Item_Ring_Forest.event.root.ainb");
const FIRE: &[u8] = include_bytes!("../../../data/ring_events/Item_Ring_Fire.event.root.ainb");
const WATER: &[u8] = include_bytes!("../../../data/ring_events/Item_Ring_Water.event.root.ainb");
const SHADOW: &[u8] = include_bytes!("../../../data/ring_events/Item_Ring_Shadow.event.root.ainb");
const SPIRIT: &[u8] = include_bytes!("../../../data/ring_events/Item_Ring_Spirit.event.root.ainb");
const WELL: &[u8] = include_bytes!("../../../data/ring_events/Item_Ring_Well.event.root.ainb");
const GTG: &[u8] = include_bytes!("../../../data/ring_events/Item_Ring_GTG.event.root.ainb");
const GANON: &[u8] = include_bytes!("../../../data/ring_events/Item_Ring_Ganon.event.root.ainb");
const SET_FLAG: &str = "OneShotSetNonStructGameDataBool";

fn ring(actor: &str) -> Res<&'static crate::key_rings::Ring> {
    crate::key_rings::all_with_hideout()
        .into_iter()
        .find(|r| r.actor == actor)
        .ok_or_else(|| E::Msg(format!("ring event: unknown actor {actor}")))
}

fn template(actor: &str) -> Res<&'static [u8]> {
    Ok(match actor {
        "Item_Ring_Forest" => FOREST,
        "Item_Ring_Fire" => FIRE,
        "Item_Ring_Water" => WATER,
        "Item_Ring_Shadow" => SHADOW,
        "Item_Ring_Spirit" => SPIRIT,
        "Item_Ring_Well" => WELL,
        "Item_Ring_GTG" => GTG,
        "Item_Ring_Ganon" => GANON,
        "Item_Ring_Hideout" => FOREST,
        _ => return Err(E::Msg(format!("ring event: unknown actor {actor}"))),
    })
}

/// The source event names the long `Item_KeyRing_<Dungeon>` actors. Our saved pouch identity is
/// the short `Item_Ring_<Dungeon>` actor, so patch both names before shipping the event.
fn legacy(actor: &str) -> Res<&'static str> {
    Ok(match actor {
        "Item_Ring_Forest" => "ForestTemple",
        "Item_Ring_Fire" => "FireTemple",
        "Item_Ring_Water" => "WaterTemple",
        "Item_Ring_Shadow" => "ShadowTemple",
        "Item_Ring_Spirit" => "SpiritTemple",
        "Item_Ring_Well" => "Well",
        "Item_Ring_GTG" => "GTG",
        "Item_Ring_Ganon" => "Ganon",
        // The short Hideout name fits the Forest template's longer strings.
        "Item_Ring_Hideout" => "ForestTemple",
        _ => return Err(E::Msg(format!("ring event: unknown actor {actor}"))),
    })
}

fn replace(raw: &[u8], old: &[u8], new: &[u8]) -> Res<Vec<u8>> {
    if new.len() > old.len() {
        return Err(E::Format("ring event: a new name is longer than its template name"));
    }
    let mut out: Vec<u8> = raw.to_vec();
    let mut from: usize = 0;
    let mut n: usize = 0;
    while let Some(at) = out[from..].windows(old.len()).position(|w| w == old) {
        let at: usize = from + at;
        let end: usize = at + old.len();
        if out.get(end) != Some(&0) {
            from = end;
            continue;
        }
        out[at..at + new.len()].copy_from_slice(new);
        out[at + new.len()..end].fill(0);
        from = end;
        n += 1;
    }
    if n == 0 {
        return Err(E::Format("ring event: the template is missing an event name"));
    }
    Ok(out)
}

/// Guard the build against ever reintroducing AINB credit. A dungeon int write here would not take
/// effect, and an applied-flag write would make the real flow credit skip itself.
fn no_direct_credit(a: &Ainb, actor: &str) -> Res<()> {
    let r: &crate::key_rings::Ring = ring(actor)?;
    let applied: String = crate::key_rings::applied_flag(r.folder);
    for kind in ["OneShotSetNonStructGameDataInt", "OneShotSetStructGameDataInt"] {
        for i in a.find_els(kind) {
            if let Some(name) = a.in_str(i, "GameDataFullName") {
                if name.starts_with("DungeonInfo.") {
                    return Err(E::Format("ring event: the AINB must not write dungeon keys"));
                }
            }
        }
    }
    for i in a.find_els(SET_FLAG) {
        if a.in_str(i, "GameDataFullName") == Some(applied.as_str()) {
            return Err(E::Format("ring event: the AINB must not set the applied flag"));
        }
    }
    Ok(())
}

/// The ring-specific get event, renamed to save `IsGet.<actor>` for our pack identity. All key credit
/// is done by the pickup flows, so this event must leave `DungeonInfo.*` and the applied flag alone.
pub fn make(actor: &str) -> Res<Vec<u8>> {
    let old: &str = legacy(actor)?;
    let mut raw: Vec<u8> = template(actor)?.to_vec();
    raw = replace(&raw, format!("IsGet.Item_KeyRing_{old}").as_bytes(), crate::key_rings::got_flag(actor).as_bytes())?;
    raw = replace(&raw, format!("RandoKey_KeyRing_{old}.event.root").as_bytes(), format!("{actor}.event.root").as_bytes())?;

    let a: Ainb = Ainb::parse(&raw)?;
    let flag: String = crate::key_rings::got_flag(actor);
    let set: Vec<usize> = a
        .find_els(SET_FLAG)
        .into_iter()
        .filter(|&i| a.in_str(i, "GameDataFullName") == Some(flag.as_str()))
        .collect();
    let get: Vec<usize> = a
        .find_els("QueryGetNonStructGameDataBool")
        .into_iter()
        .filter(|&i| a.in_str(i, "GameDataFullName") == Some(flag.as_str()))
        .collect();
    if set.len() != 1 || get.len() != 1 {
        return Err(E::Format("ring event: the template lost its saved flag nodes"));
    }
    no_direct_credit(&a, actor)?;
    let bad: Vec<String> = audit(&a);
    if !bad.is_empty() {
        return Err(E::Msg(format!("ring event: {actor}: {}", bad.join("; "))));
    }
    a.write()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ring_event_sets_its_own_saved_flag() {
        for r in crate::key_rings::all_with_hideout() {
            let raw: Vec<u8> = make(r.actor).unwrap();
            let a: Ainb = Ainb::parse(&raw).unwrap();
            let flag: String = crate::key_rings::got_flag(r.actor);
            let set: Vec<usize> = a
                .find_els(SET_FLAG)
                .into_iter()
                .filter(|&i| a.in_str(i, "GameDataFullName") == Some(flag.as_str()))
                .collect();
            let get: Vec<usize> = a
                .find_els("QueryGetNonStructGameDataBool")
                .into_iter()
                .filter(|&i| a.in_str(i, "GameDataFullName") == Some(flag.as_str()))
                .collect();
            assert_eq!((set.len(), get.len()), (1, 1), "{}", r.actor);
            assert!(!String::from_utf8_lossy(&raw).contains("Item_KeyRing_"), "{}", r.actor);
        }
    }

    #[test]
    fn every_ring_event_leaves_key_credit_to_the_flows() {
        for r in crate::key_rings::all_with_hideout() {
            let raw: Vec<u8> = make(r.actor).unwrap();
            let a: Ainb = Ainb::parse(&raw).unwrap();
            for name in [format!("DungeonInfo.{}.SmallKeyNum", r.folder), format!("DungeonInfo.{}.SmallKeyTotalNum", r.folder)] {
                for kind in ["OneShotSetNonStructGameDataInt", "OneShotSetStructGameDataInt"] {
                    let set: Vec<usize> = a
                        .find_els(kind)
                        .into_iter()
                        .filter(|&i| a.in_str(i, "GameDataFullName") == Some(name.as_str()))
                        .collect();
                    assert!(set.is_empty(), "{}: {} must not be written by the AINB", r.actor, name);
                }
            }
            let applied: String = crate::key_rings::applied_flag(r.folder);
            let set: Vec<usize> = a
                .find_els(SET_FLAG)
                .into_iter()
                .filter(|&i| a.in_str(i, "GameDataFullName") == Some(applied.as_str()))
                .collect();
            assert!(set.is_empty(), "{}: {} must not be written by the AINB", r.actor, applied);
            assert!(audit(&a).is_empty(), "{}: bad AINB", r.actor);
            let again: Ainb = Ainb::parse(&a.write().unwrap()).unwrap();
            assert!(audit(&again).is_empty(), "{}: bad AINB after round trip", r.actor);
        }
    }
}
