//! Gives every small key of a dungeon when its key ring is picked up.
//!
//! Every item get ends by calling into `ActionGuide`. The credit checks are hung off the two
//! `ActionGuide` entries used by item gets and shops, after the get flow has added the ring to the
//! pouch and written its saved `IsGet` bool. Checking at the pouch add itself read the previous
//! pickup's flag and credited one pickup late. Each held ring adds its dungeon's full `SmallKeyNum`
//! and `SmallKeyTotalNum` once, guarded by a saved applied flag, then names the dungeon. The `IsGet`
//! struct also gets one member per ring actor, so the game's own pouch add records that saved bool
//! for a generated actor it has never heard of.
//!
//! Chests use their own `GetEventName` credit (crate::dungeon_credit) and gift scenes use
//! crate::gift_keys, because neither goes through the shared get flows. Their applied flags keep
//! these deferred checks from crediting the same ring twice.

use crate::bfevfl::{Bf, Case, Cont, Event, Flow, Kind, Val};
use crate::byml::{Byml, Node};
use crate::chest_slot::LIST;
use crate::error::{Error as E, Res};
use crate::flow_locations::dict_id;
use crate::romfs::Romfs;
use crate::staging::{rewrite, Out};
use crate::zs;

const PREFIX: &str = "OOTRRKeyRing";
const FLOW: &str = "ActionGuide";
/// The no-tutorials patch leaves these two call sites for the ring checks. Both run after the item
/// has been added to the pouch: one for ordinary gets (pots, crates, ground spots) and one for shops.
const CALLS: [(&str, &str); 2] = [("ActionGuide", "OOTRRRingCallItemGet"), ("ActionGuideForShop", "OOTRRRingCallForShop")];
const HOLDER: &str = "GameDataActor";
const ASK_BOOL: &str = "EventQueryIsGameDataBool";
const ADD: &str = "EventTriggerAddOrSubGameDataInt";
const SET: &str = "EventTriggerSetGameDataBool";
const BASE_ROW: u32 = 0x154ef588;

fn idx(n: usize) -> Res<u16> {
    u16::try_from(n).map_err(|_| E::Format("key rings: too many events in the flow"))
}

fn push(f: &mut Flow, kind: Kind) -> Res<u16> {
    let i: u16 = idx(f.events.len())?;
    f.events.push(Event { name: format!("{PREFIX}{i}"), kind });
    Ok(i)
}

/// A bool query answers `0` for true and `1` for false, so the case values name the answer, not the
/// meaning. Callers pass the event for a true answer first and the event for a false answer second.
fn switch(f: &mut Flow, actor: &str, query: &str, params: Cont, on_true: u16, on_false: u16) -> Res<Kind> {
    let (a, q): (u16, u16) = crate::start_state::query_slot(f, actor, query)?;
    Ok(Kind::Switch { actor: a, query: q, cases: vec![Case { val: 0, ev: on_true }, Case { val: 1, ev: on_false }], params: Some(params) })
}

fn add_int(name: String, count: u32) -> Cont {
    Cont {
        items: vec![
            ("Index".to_string(), Val::Int(-1)),
            ("GameDataName".to_string(), Val::Str(name)),
            ("Value".to_string(), Val::Int(count as i32)),
        ],
    }
}

/// One ring's guarded credit, ending on `next`. The applied flag is checked first, then the saved
/// `IsGet` bool, so a ring that was already credited (or is not the item being picked up) falls
/// straight through.
fn ring(f: &mut Flow, r: &'static crate::key_rings::Ring, next: u16) -> Res<u16> {
    let applied: String = crate::key_rings::applied_flag(r.folder);
    let (a, add): (u16, u16) = crate::dungeon_credit::action_slot(f, HOLDER, ADD)?;
    let (_, set): (u16, u16) = crate::dungeon_credit::action_slot(f, HOLDER, SET)?;
    let (t, say): (u16, u16) = crate::dungeon_credit::action_slot(f, crate::dungeon_credit::TALKER, crate::dungeon_credit::TALK)?;

    let say_it: u16 = push(f, Kind::Action {
        next,
        actor: t,
        act: say,
        params: Some(crate::dungeon_credit::talk(crate::dungeon_text::message_id(r.folder))),
    })?;
    let mark: u16 = push(f, Kind::Action { next: say_it, actor: a, act: set, params: Some(crate::dungeon_credit::set_bool(applied.clone())) })?;
    let once: u16 = push(f, Kind::Action {
        next: mark,
        actor: a,
        act: set,
        params: Some(crate::dungeon_credit::set_bool(crate::dungeon_credit::flag(r.folder, "IsGetSmallKeyOnce"))),
    })?;
    let total: u16 = push(f, Kind::Action {
        next: once,
        actor: a,
        act: add,
        params: Some(add_int(crate::dungeon_credit::flag(r.folder, "SmallKeyTotalNum"), r.count)),
    })?;
    let num: u16 = push(f, Kind::Action {
        next: total,
        actor: a,
        act: add,
        params: Some(add_int(crate::dungeon_credit::flag(r.folder, "SmallKeyNum"), r.count)),
    })?;
    // The saved `IsGet.<actor>` bool answers 0 when the ring is held, so credit. The pouch query is
    // unreliable for generated key items: in game the ring can be in the pouch while the pouch query
    // still says no, which was exactly the world-drop ring's missing-credit symptom.
    let held: Kind = switch(
        f,
        HOLDER,
        ASK_BOOL,
        Cont { items: vec![("GameDataName".to_string(), Val::Str(crate::key_rings::got_flag(r.actor))), ("Index".to_string(), Val::Int(-1))] },
        num,
        next,
    )?;
    let held_q: u16 = push(f, held)?;
    // Already applied: answer 0, so skip. Not applied yet: answer 1, check the saved held flag.
    let asked: Kind = switch(
        f,
        HOLDER,
        ASK_BOOL,
        Cont { items: vec![("GameDataName".to_string(), Val::Str(applied)), ("Index".to_string(), Val::Int(-1))] },
        next,
        held_q,
    )?;
    push(f, asked)
}

/// Hangs the ring checks off the two `ActionGuide` call sites the no-tutorials patch leaves. Each
/// chain is built in reverse, so an unmatched or already credited ring falls through to the call's
/// one-frame wait. The entries start at the new chains instead of at the waits.
pub fn patch(f: &mut Flow) -> Res<()> {
    if f.events.iter().any(|e| e.name.starts_with(PREFIX)) {
        return Ok(());
    }
    for (entry_name, call_name) in CALLS {
        let call: usize = f.events.iter().position(|e| e.name == call_name).ok_or_else(|| E::Msg(format!("key rings: no {call_name} event in the {FLOW} flow")))?;
        if !matches!(f.events[call].kind, Kind::Action { .. }) {
            return Err(E::Msg(format!("key rings: {call_name} is not an action, the game files differ")));
        }
        let wait: u16 = idx(call)?;
        let entry: &crate::bfevfl::Entry = f.entries.iter().find(|e| e.name == entry_name).ok_or_else(|| E::Msg(format!("key rings: no {entry_name} entry in the {FLOW} flow")))?;
        if entry.main != wait {
            return Err(E::Msg(format!("key rings: {entry_name} does not start at {call_name}, the game files differ")));
        }
        let mut first: u16 = wait;
        for r in crate::key_rings::RINGS.iter().rev() {
            first = ring(f, r, first)?;
        }
        f.entries.iter_mut().find(|e| e.name == entry_name).unwrap().main = first;
    }
    Ok(())
}

pub fn patch_bf(b: &mut Bf) -> Res<()> {
    let f: &mut Flow = b.flows.iter_mut().find(|x| x.name == FLOW).ok_or_else(|| E::Msg(format!("key rings: no {FLOW} flow")))?;
    patch(f)
}

/// Stages the credit on top of the no-tutorials patch. It reads the file that stage already wrote
/// into the mod's work folder, so the ring checks keep the two separate item-get call sites.
pub fn stage_flows(rf: &mut Romfs, late: &mut Vec<Out>, work: &std::path::Path) -> Res<()> {
    crate::story_skip::stage_late(rf, late, work, FLOW, patch_bf)
}

fn flag_pairs() -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for r in crate::key_rings::RINGS {
        let got: String = crate::key_rings::got_flag(r.actor);
        let applied: String = crate::key_rings::applied_flag(r.folder);
        out.push((got.clone(), crate::flags::hash_name(&got)));
        out.push((applied.clone(), crate::flags::hash_name(&applied)));
    }
    out
}

fn row_hash(r: &Node) -> Option<u32> {
    match r.get("Hash") {
        Some(Node::U32(h)) => Some(*h),
        _ => None,
    }
}

fn is_our_row(r: &Node, h: u32) -> bool {
    row_hash(r) == Some(h)
        && r.get("DefaultValue") == Some(&Node::Bool(false))
        && r.get("ResetTypeValue") == Some(&Node::I32(8))
        && r.get("SaveFileIndex") == Some(&Node::I32(0))
}

/// The native pouch add writes `IsGet.<actor>` through the `IsGet` struct: member `hash(actor)`
/// points at the saved Bool `hash("IsGet." + actor)`. A generated ring has no vanilla struct member,
/// so without this the get event can put the ring in the pouch and still leave our held check false.
fn add_isget_members(y: &mut Byml) -> Res<()> {
    let isget: u32 = crate::flags::hash_name("IsGet");
    let structs: &mut Vec<Node> = y
        .root
        .get_mut("Data")
        .and_then(|d| d.get_mut("Struct"))
        .and_then(|s| s.as_arr_mut())
        .ok_or_else(|| E::Format("key rings: the game data list has no Struct table"))?;
    let members: &mut Vec<Node> = structs
        .iter_mut()
        .find(|s| row_hash(s) == Some(isget))
        .and_then(|s| s.get_mut("DefaultValue"))
        .and_then(|v| v.as_arr_mut())
        .ok_or_else(|| E::Format("key rings: the game data list has no IsGet struct"))?;
    for r in crate::key_rings::RINGS {
        let member: u32 = crate::flags::hash_name(r.actor);
        let flag: u32 = crate::flags::hash_name(&crate::key_rings::got_flag(r.actor));
        if let Some(row) = members.iter().find(|m| row_hash(m) == Some(member)) {
            if row.get("Value") != Some(&Node::U32(flag)) {
                return Err(E::Msg(format!("key rings: the IsGet member for {} differs from the game data", r.actor)));
            }
            continue;
        }
        members.push(Node::Map(vec![
            ("Hash".to_string(), Node::U32(member)),
            ("Value".to_string(), Node::U32(flag)),
        ]));
    }
    Ok(())
}

fn add_rows(y: &mut Byml) -> Res<()> {
    let rows: &mut Vec<Node> = y
        .root
        .get_mut("Data")
        .and_then(|d| d.get_mut("Bool"))
        .and_then(|b| b.as_arr_mut())
        .ok_or_else(|| E::Format("key rings: the game data list has no Bool table"))?;
    let base: Node = rows
        .iter()
        .find(|r| row_hash(r) == Some(BASE_ROW))
        .cloned()
        .ok_or_else(|| E::Format("key rings: the game data list has no saved Bool template"))?;
    for (name, h) in flag_pairs() {
        if let Some(row) = rows.iter().find(|r| row_hash(r) == Some(h)) {
            if !is_our_row(row, h) {
                return Err(E::Msg(format!("key rings: the flag {name} clashes with a game flag")));
            }
            continue;
        }
        let mut row: Node = base.clone();
        row.set("Hash", Node::U32(h));
        row.set("DefaultValue", Node::Bool(false));
        row.set("ResetTypeValue", Node::I32(8));
        row.set("SaveFileIndex", Node::I32(0));
        rows.push(row);
    }
    Ok(())
}

pub fn add_runtime_rows(raw: &[u8]) -> Res<Vec<u8>> {
    let mut y: Byml = Byml::parse(raw)?;
    add_rows(&mut y)?;
    add_isget_members(&mut y)?;
    y.write()
}

fn edit_raw(raw: &[u8]) -> Res<Vec<u8>> {
    add_runtime_rows(raw)
}

/// Stages the saved Bools the credit checks use and the `IsGet` members that make the game write
/// them. Later GameDataList editors merge with this file.
pub fn stage_rows(rf: &mut Romfs, more: &mut Vec<Out>, enabled: bool) -> Res<()> {
    if !enabled {
        return Ok(());
    }
    match more.iter().position(|o| o.path == LIST) {
        Some(i) => {
            let id: Option<u32> = dict_id(&more[i].z);
            let raw: Vec<u8> = rf.zs.decompress(&more[i].z)?;
            let out: Vec<u8> = edit_raw(&raw)?;
            more[i].new = out.len();
            more[i].z = zs::compress_raw(&out, id)?;
        }
        None => more.push(rewrite(rf, LIST, edit_raw)?),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bfevfl::flow::NONE;
    use crate::bfevfl::{Actor, Entry};
    use std::collections::{BTreeMap as BM, BTreeSet as BS};

    const EXPECTED: [(&str, u32); 19] = [
        ("test", 0xba6bd213),
        ("DungeonInfo.ForestTemple.SmallKeyNum", 0x90237092),
        ("IsGet.Item_Dungeon_SmallKey", 0x154ef588),
        ("IsGet.Item_Ring_Forest", 0x2299468c),
        ("IsGet.Item_Ring_Fire", 0x223b086e),
        ("IsGet.Item_Ring_Water", 0xd8db874b),
        ("IsGet.Item_Ring_Shadow", 0x92d9860a),
        ("IsGet.Item_Ring_Spirit", 0xb28490ba),
        ("IsGet.Item_Ring_Well", 0xd5e3c18d),
        ("IsGet.Item_Ring_GTG", 0x5f62ae19),
        ("IsGet.Item_Ring_Ganon", 0x685bed59),
        ("OOTRR_KeyRing.ForestTemple", 0x2cffcc4f),
        ("OOTRR_KeyRing.FireTemple", 0x4e79f9b5),
        ("OOTRR_KeyRing.WaterTemple", 0xeba9f19f),
        ("OOTRR_KeyRing.ShadowTemple", 0xc8d7d030),
        ("OOTRR_KeyRing.SpiritTemple", 0x64576046),
        ("OOTRR_KeyRing.BottomOfTheWell", 0x0607308e),
        ("OOTRR_KeyRing.GerudoTrainingGround", 0x26517f00),
        ("OOTRR_KeyRing.InsideGanonCastle", 0x97150d07),
    ];

    fn actor(name: &str, acts: &[&str], queries: &[&str]) -> Actor {
        Actor {
            name: name.to_string(),
            sub: String::new(),
            arg: String::new(),
            actions: acts.iter().map(|s| s.to_string()).collect(),
            queries: queries.iter().map(|s| s.to_string()).collect(),
            params: None,
            ep: 0,
            cut: 0,
        }
    }

    fn calls() -> [(u16, &'static str, &'static str, &'static str); 2] {
        [
            (1, "ActionGuide", "OOTRRRingCallItemGet", "the item get"),
            (2, "ActionGuideForShop", "OOTRRRingCallForShop", "the shop get"),
        ]
    }

    fn action_guide() -> Flow {
        Flow {
            name: FLOW.to_string(),
            actors: vec![actor("EventSystemActor", &["EventWait"], &[])],
            events: vec![
                Event {
                    name: "OOTRRNoGuide0".to_string(),
                    kind: Kind::Action { next: NONE, actor: 0, act: 0, params: None },
                },
                Event {
                    name: "OOTRRRingCallItemGet".to_string(),
                    kind: Kind::Action { next: 0, actor: 0, act: 0, params: None },
                },
                Event {
                    name: "OOTRRRingCallForShop".to_string(),
                    kind: Kind::Action { next: 0, actor: 0, act: 0, params: None },
                },
            ],
            entries: vec![
                Entry { name: "ActionGuide".to_string(), subs: Vec::new(), vars: Vec::new(), main: 1, pad: 0 },
                Entry { name: "ActionGuideForShop".to_string(), subs: Vec::new(), vars: Vec::new(), main: 2, pad: 0 },
            ],
        }
    }

    #[test]
    fn every_item_get_checks_each_held_ring_before_its_guide_wait() {
        let mut f: Flow = action_guide();
        patch(&mut f).unwrap();
        for (wait, entry_name, call_name, label) in calls() {
            let entry: &Entry = f.entries.iter().find(|e| e.name == entry_name).unwrap();
            let mut at: usize = entry.main as usize;
            assert_ne!(at, wait as usize, "{label} starts at the ring checks, not the wait");
            assert_eq!(f.events[wait as usize].name, call_name);
            let mut applied_seen: BS<String> = BS::new();
            let mut held_seen: BS<String> = BS::new();
            let mut reached_wait: bool = false;
            for _ in 0..f.events.len() {
                if at == wait as usize {
                    reached_wait = true;
                    break;
                }
                let e: &Event = &f.events[at];
                match &e.kind {
                    Kind::Switch { params, cases, .. } => {
                        let c: &Cont = params.as_ref().unwrap();
                        let Some(Val::Str(name)) = c.get("GameDataName") else { panic!("a ring switch needs a flag") };
                        if name.starts_with("OOTRR_KeyRing.") {
                            applied_seen.insert(name.clone());
                        } else if name.starts_with("IsGet.") {
                            held_seen.insert(name.clone());
                        }
                        // An applied flag is false on a first pickup (answer 1), while the saved
                        // IsGet bool is true once the ring is held (answer 0).
                        let answer: u32 = if name.starts_with("OOTRR_KeyRing.") { 1 } else { 0 };
                        at = cases.iter().find(|c| c.val == answer).unwrap().ev as usize;
                    }
                    Kind::Action { next, .. } => at = *next as usize,
                    _ => panic!("unexpected ring credit event"),
                }
            }
            assert!(reached_wait, "{label} reaches its wait");
            assert_eq!(applied_seen.len(), 8, "{label} has an applied-flag guard per ring");
            assert_eq!(held_seen.len(), 8, "{label} checks every saved IsGet bool");
            assert!(applied_seen.contains(&crate::key_rings::applied_flag("ForestTemple")));
            assert!(held_seen.contains(&crate::key_rings::got_flag("Item_Ring_Forest")));
        }
        assert!(f.events.iter().any(|e| e.name.starts_with(PREFIX)));
        let before: Flow = f.clone();
        patch(&mut f).unwrap();
        assert_eq!(f, before, "the credit is added once");
    }

    #[test]
    fn a_picked_up_ring_writes_its_full_count_and_its_applied_flag() {
        let mut f: Flow = action_guide();
        patch(&mut f).unwrap();
        let forest: &crate::key_rings::Ring = crate::key_rings::by_folder("ForestTemple").unwrap();
        let applied: String = crate::key_rings::applied_flag(forest.folder);
        let (wait, entry_name, _, _) = calls()[0];
        let mut at: usize = f.entries.iter().find(|e| e.name == entry_name).unwrap().main as usize;
        let mut credit: Option<u16> = None;
        for _ in 0..f.events.len() {
            if at == wait as usize {
                break;
            }
            let e: &Event = &f.events[at];
            if let Kind::Switch { cases, params, .. } = &e.kind
                && params.as_ref().and_then(|c| c.get("GameDataName")) == Some(&Val::Str(applied.clone()))
            {
                let skip: u16 = cases.iter().find(|c| c.val == 0).unwrap().ev;
                credit = Some(cases.iter().find(|c| c.val == 1).unwrap().ev);
                assert_ne!(skip, credit.unwrap(), "an applied flag has its own branch");
                break;
            }
            let Kind::Action { next, .. } = &e.kind else { panic!("expected a ring check") };
            at = *next as usize;
        }
        let mut at: usize = credit.expect("the Forest applied flag is checked once per chain") as usize;
        let mut writes: BM<String, i32> = BM::new();
        for _ in 0..f.events.len() {
            let e: &Event = &f.events[at];
            match &e.kind {
                Kind::Switch { cases, params, .. } => {
                    // The saved IsGet switch credits when true (answer 0); an applied flag skips
                    // when true (answer 0), so its credit branch is answer 1.
                    let name: String = match params.as_ref().and_then(|c| c.get("GameDataName")) {
                        Some(Val::Str(n)) => n.clone(),
                        _ => panic!("a ring switch needs a flag"),
                    };
                    let answer: u32 = if name.starts_with("OOTRR_KeyRing.") { 1 } else { 0 };
                    at = cases.iter().find(|c| c.val == answer).unwrap().ev as usize;
                }
                Kind::Action { next, params, .. } => {
                    if let Some(c) = params
                        && let Some(Val::Str(name)) = c.get("GameDataName")
                    {
                        let value: Option<i32> = match c.get("Value") {
                            Some(Val::Int(v)) => Some(*v),
                            Some(Val::Bool(v)) => Some(*v as i32),
                            _ => None,
                        };
                        if let Some(v) = value {
                            writes.insert(name.clone(), v);
                        }
                    }
                    if *next == wait {
                        break;
                    }
                    at = *next as usize;
                }
                _ => break,
            }
        }
        assert_eq!(writes.get("DungeonInfo.ForestTemple.SmallKeyNum"), Some(&5));
        assert_eq!(writes.get("DungeonInfo.ForestTemple.SmallKeyTotalNum"), Some(&5));
        assert_eq!(writes.get("DungeonInfo.ForestTemple.IsGetSmallKeyOnce"), Some(&1));
        assert_eq!(writes.get(applied.as_str()), Some(&1));
    }

    #[test]
    fn every_flag_the_ring_credit_uses_has_a_staged_game_data_row() {
        let names: BS<String> = flag_pairs().into_iter().map(|(n, _)| n).collect();
        for r in crate::key_rings::RINGS {
            assert!(names.contains(&crate::key_rings::got_flag(r.actor)));
            assert!(names.contains(&crate::key_rings::applied_flag(r.folder)));
        }
        for (n, h) in EXPECTED {
            assert_eq!(crate::flags::hash_name(n), h, "{n}");
        }
    }

    #[test]
    fn the_pouch_add_can_record_each_ring_through_the_is_get_struct() {
        let mut y: Byml = Byml {
            root: Node::Map(vec![(
                "Data".to_string(),
                Node::Map(vec![
                    ("Bool".to_string(), Node::Arr(vec![Node::Map(vec![("Hash".to_string(), Node::U32(BASE_ROW))])])),
                    (
                        "Struct".to_string(),
                        Node::Arr(vec![Node::Map(vec![
                            ("Hash".to_string(), Node::U32(crate::flags::hash_name("IsGet"))),
                            ("DefaultValue".to_string(), Node::Arr(Vec::new())),
                        ])]),
                    ),
                ]),
            )]),
            version: 2,
            big: false,
        };
        add_rows(&mut y).unwrap();
        add_isget_members(&mut y).unwrap();
        let bools: &[Node] = y.root.get("Data").unwrap().get("Bool").unwrap().as_arr().unwrap();
        assert_eq!(bools.len(), 1 + 16, "each ring has a held flag and an applied flag");
        let isget: u32 = crate::flags::hash_name("IsGet");
        let members: &[Node] = y
            .root
            .get("Data")
            .unwrap()
            .get("Struct")
            .unwrap()
            .as_arr()
            .unwrap()
            .iter()
            .find(|s| row_hash(s) == Some(isget))
            .unwrap()
            .get("DefaultValue")
            .unwrap()
            .as_arr()
            .unwrap();
        assert_eq!(members.len(), 8, "each generated ring actor gets one IsGet member");
        for r in crate::key_rings::RINGS {
            let member: u32 = crate::flags::hash_name(r.actor);
            let flag: u32 = crate::flags::hash_name(&crate::key_rings::got_flag(r.actor));
            assert_eq!(
                members.iter().find(|m| row_hash(m) == Some(member)).and_then(|m| m.get("Value")),
                Some(&Node::U32(flag)),
                "{}",
                r.actor
            );
        }
        let before: Byml = y.clone();
        add_rows(&mut y).unwrap();
        add_isget_members(&mut y).unwrap();
        assert_eq!(y, before, "the rows are added once");
    }
}
