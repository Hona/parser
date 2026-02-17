#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]
#![cfg_attr(not(test), deny(clippy::panic))]

use bitbuffer::BitRead;
pub use bitbuffer::Result as ReadResult;
use js_sys::{Array, Object, Reflect, Uint16Array, Uint32Array, Uint8Array};
use serde::{Deserialize, Serialize};
use serde_wasm_bindgen::to_value;
use std::collections::BTreeMap;
use std::ffi::{c_char, CStr, CString};
use std::fs;
use wasm_bindgen::prelude::*;

use crate::demo::data::game_state::{PlayerClassData, PlayerState, ProjectileType};
use crate::demo::data::DemoTick;
use crate::demo::header::Header;
use crate::demo::message::EntityId;
use crate::demo::parser::analyser::{Analyser as MatchStateAnalyser, Team as GameTeam};
use crate::demo::parser::gamestateanalyser::GameStateAnalyser;
use crate::demo::parser::handler::{BorrowMessageHandler, DemoHandler};
use crate::demo::parser::{MessageHandler, RawPacketStream};
pub use crate::demo::{
    message::MessageType,
    parser::{
        DemoParser, GameEventError, MatchState, MessageTypeAnalyser, Parse, ParseError,
        ParserState, Result,
    },
    Demo, Stream,
};

#[cfg(feature = "codegen")]
pub mod codegen;
pub(crate) mod consthash;
pub mod demo;
pub(crate) mod nullhasher;

#[cfg(all(test, feature = "write"))]
#[track_caller]
fn test_roundtrip_write<
    'a,
    T: bitbuffer::BitRead<'a, bitbuffer::LittleEndian>
        + bitbuffer::BitWrite<bitbuffer::LittleEndian>
        + std::fmt::Debug
        + std::cmp::PartialEq,
>(
    val: T,
) {
    let mut data = Vec::with_capacity(128);
    use bitbuffer::{BitReadBuffer, BitReadStream, BitWriteStream, LittleEndian};
    let pos = {
        let mut stream = BitWriteStream::new(&mut data, LittleEndian);
        val.write(&mut stream).unwrap();
        stream.bit_len()
    };

    let mut read = BitReadStream::new(BitReadBuffer::new_owned(data, LittleEndian));
    assert_eq!(
        val,
        read.read().unwrap(),
        "Failed to assert the parsed message is equal to the original"
    );
    assert_eq!(
        pos,
        read.pos(),
        "Failed to assert that all encoded bits ({}) are used for decoding ({})",
        pos,
        read.pos()
    );
}

#[cfg(all(test, feature = "write"))]
#[track_caller]
fn test_roundtrip_encode<
    'a,
    T: Parse<'a> + crate::demo::parser::Encode + std::fmt::Debug + std::cmp::PartialEq,
>(
    val: T,
    state: &ParserState,
) {
    let mut data = Vec::with_capacity(128);
    use bitbuffer::{BitReadBuffer, BitReadStream, BitWriteStream, LittleEndian};
    let pos = {
        let mut stream = BitWriteStream::new(&mut data, LittleEndian);
        val.encode(&mut stream, state).unwrap();
        stream.bit_len()
    };

    let mut read = BitReadStream::new(BitReadBuffer::new_owned(data, LittleEndian));
    pretty_assertions::assert_eq!(
        val,
        T::parse(&mut read, state).unwrap(),
        "Failed to assert the parsed message is equal to the original"
    );
    pretty_assertions::assert_eq!(
        pos,
        read.pos(),
        "Failed to assert that all encoded bits ({}) are used for decoding ({})",
        pos,
        read.pos()
    );
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonDemo {
    header: Header,
    #[serde(flatten)]
    state: MatchState,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsWorld {
    boundary_min: crate::demo::vector::Vector,
    boundary_max: crate::demo::vector::Vector,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayerRef {
    user: crate::demo::parser::analyser::UserInfo,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CachedDeathEntry {
    tick: u32,
    victim: PlayerRef,
    assister: Option<PlayerRef>,
    killer: Option<PlayerRef>,
    weapon: String,
    victim_team: u8,
    assister_team: u8,
    killer_team: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RoundEntry {
    winner: String,
    length: f32,
    end_tick: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatEntry {
    tick: u32,
    kind: crate::demo::message::usermessage::ChatMessageKind,
    client_entity_id: Option<u32>,
    client_player_id: Option<u32>,
    raw_text: String,
}

#[derive(Debug)]
struct CombinedOutput {
    match_state: MatchState,
}

#[derive(Debug, Default)]
struct CombinedAnalyser {
    match_state: MatchStateAnalyser,
    game_state: GameStateAnalyser,
}

impl CombinedAnalyser {
    fn match_state(&self, parser_state: &ParserState) -> &MatchState {
        self.match_state.borrow_output(parser_state)
    }

    fn game_state(&self, parser_state: &ParserState) -> &crate::demo::data::game_state::GameState {
        self.game_state.borrow_output(parser_state)
    }
}

impl MessageHandler for CombinedAnalyser {
    type Output = CombinedOutput;

    fn does_handle(message_type: MessageType) -> bool {
        MatchStateAnalyser::does_handle(message_type)
            || GameStateAnalyser::does_handle(message_type)
    }

    fn handle_message(
        &mut self,
        message: &crate::demo::message::Message,
        tick: DemoTick,
        parser_state: &ParserState,
    ) {
        if MatchStateAnalyser::does_handle(message.get_message_type()) {
            self.match_state.handle_message(message, tick, parser_state);
        }
        if GameStateAnalyser::does_handle(message.get_message_type()) {
            self.game_state.handle_message(message, tick, parser_state);
        }
    }

    fn handle_string_entry(
        &mut self,
        table: &str,
        index: usize,
        entry: &crate::demo::packet::stringtable::StringTableEntry,
        parser_state: &ParserState,
    ) {
        self.match_state
            .handle_string_entry(table, index, entry, parser_state);
        self.game_state
            .handle_string_entry(table, index, entry, parser_state);
    }

    fn handle_data_tables(
        &mut self,
        tables: &[crate::demo::packet::datatable::ParseSendTable],
        server_classes: &[crate::demo::packet::datatable::ServerClass],
        parser_state: &ParserState,
    ) {
        self.match_state
            .handle_data_tables(tables, server_classes, parser_state);
        self.game_state
            .handle_data_tables(tables, server_classes, parser_state);
    }

    fn handle_packet_meta(
        &mut self,
        tick: DemoTick,
        meta: &crate::demo::packet::message::MessagePacketMeta,
        parser_state: &ParserState,
    ) {
        self.match_state
            .handle_packet_meta(tick, meta, parser_state);
        self.game_state.handle_packet_meta(tick, meta, parser_state);
    }

    fn into_output(self, state: &ParserState) -> Self::Output {
        CombinedOutput {
            match_state: self.match_state.into_output(state),
        }
    }
}

#[no_mangle]
pub extern "C" fn analyze_demo(path: *const c_char) -> *mut c_char {
    let c_str = unsafe { CStr::from_ptr(path) };
    let str_slice = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let file = match fs::read(str_slice) {
        Ok(f) => f,
        Err(_) => return std::ptr::null_mut(),
    };

    let demo = Demo::new(&file);
    let parser = DemoParser::new(demo.get_stream());
    let (header, state) = match parser.parse() {
        Ok(results) => results,
        Err(_) => return std::ptr::null_mut(),
    };

    let demo_json = JsonDemo { header, state };
    let result = match serde_json::to_string(&demo_json) {
        Ok(json) => json,
        Err(_) => return std::ptr::null_mut(),
    };

    let c_string = match CString::new(result) {
        Ok(c_str) => c_str,
        Err(_) => return std::ptr::null_mut(),
    };

    c_string.into_raw()
}

#[no_mangle]
pub extern "C" fn free_string(s: *mut c_char) {
    unsafe {
        if !s.is_null() {
            let _ = CString::from_raw(s);
        }
    }
}

fn map_projectile_type(ty: ProjectileType) -> u8 {
    match ty {
        ProjectileType::Rocket => 1,
        ProjectileType::Pipe => 2,
        ProjectileType::Sticky => 3,
        ProjectileType::HealingArrow => 4,
        _ => 0,
    }
}

fn map_team(team: GameTeam) -> u8 {
    match team {
        GameTeam::Red => 2,
        GameTeam::Blue => 3,
        GameTeam::Spectator => 1,
        _ => 0,
    }
}

fn pack_meta(class_id: u8, team_id: u8) -> u8 {
    (class_id & 0x0f) | ((team_id & 0x0f) << 4)
}

fn match_team_to_str(team: crate::demo::parser::analyser::Team) -> &'static str {
    match team {
        crate::demo::parser::analyser::Team::Red => "red",
        crate::demo::parser::analyser::Team::Blue => "blue",
        crate::demo::parser::analyser::Team::Spectator => "spectator",
        _ => "",
    }
}

fn match_team_to_id(team: crate::demo::parser::analyser::Team) -> u8 {
    match team {
        crate::demo::parser::analyser::Team::Red => 2,
        crate::demo::parser::analyser::Team::Blue => 3,
        crate::demo::parser::analyser::Team::Spectator => 1,
        _ => 0,
    }
}

fn to_fixed_u32(value: f32, scale: f32) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    let fixed = (value * scale).round() as i64;
    fixed as u32
}

const POSITION_FIXED_SCALE: f32 = 32.0;
const ANGLE_FIXED_SCALE: f32 = 256.0;

const TELEPORT_DISTANCE: f32 = 4096.0;
const FL_DUCKING: u32 = 1 << 1;
const FL_ANIMDUCKING: u32 = 1 << 2;

#[derive(Clone, Copy, Debug, Default)]
struct PlayerSample {
    connected: bool,
    position: crate::demo::vector::Vector,
    view_angle: f32,
    pitch_angle: f32,
    health: u16,
    class_id: u8,
    team_id: u8,
    uber: u16,
    heal_target: u16,
    flags: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct ProjectileSample {
    position: crate::demo::vector::Vector,
    rotation: crate::demo::vector::Vector,
    team_id: u8,
    ty: u8,
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_vec(
    a: crate::demo::vector::Vector,
    b: crate::demo::vector::Vector,
    t: f32,
) -> crate::demo::vector::Vector {
    crate::demo::vector::Vector {
        x: lerp(a.x, b.x, t),
        y: lerp(a.y, b.y, t),
        z: lerp(a.z, b.z, t),
    }
}

fn lerp_angle_deg(a: f32, b: f32, t: f32) -> f32 {
    if !a.is_finite() || !b.is_finite() {
        return a;
    }
    let mut delta = (b - a).rem_euclid(360.0);
    if delta > 180.0 {
        delta -= 360.0;
    }
    a + delta * t
}

fn distance(a: crate::demo::vector::Vector, b: crate::demo::vector::Vector) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let dz = b.z - a.z;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn parse_demo_cache_internal(
    buffer: &[u8],
    progress_callback: Option<&js_sys::Function>,
) -> JsValue {
    let demo = Demo::new(buffer);
    let mut stream = demo.get_stream();
    let header = match Header::read(&mut stream) {
        Ok(value) => value,
        Err(_) => return JsValue::from_str("Failed to parse demo header"),
    };
    let mut handler = DemoHandler::with_analyser(CombinedAnalyser::default());
    handler.handle_header(&header);
    let mut packets = RawPacketStream::new(stream);

    let tick_capacity = header.ticks.max(1) as usize;
    let mut start_server_tick: u32 = 0;
    let mut start_demo_tick: u32 = 0;
    let mut start_tick_base: i64 = 0;
    let mut started = false;
    let mut warmup_frames = 0u32;

    let mut tick_base_samples: u32 = 0;
    let mut tick_base_changes: u32 = 0;
    let mut tick_base_last: Option<i64> = None;
    let mut tick_base_min: i64 = 0;
    let mut tick_base_max: i64 = 0;
    let mut tick_skew_min: i64 = 0;
    let mut tick_skew_max: i64 = 0;
    let mut tick_skew_abs_max: i64 = 0;

    let mut cache_builder =
        CacheBuilder::new(tick_capacity, crate::demo::vector::Vector::default());
    let mut player_ids: BTreeMap<EntityId, usize> = BTreeMap::new();
    let mut user_info_by_entity: BTreeMap<EntityId, crate::demo::parser::analyser::UserInfo> =
        BTreeMap::new();

    let mut last_snapshot_tick: Option<u32> = None;
    let mut last_players: Vec<Option<PlayerSample>> = Vec::new();
    let mut last_projectiles: Vec<Option<ProjectileSample>> = Vec::new();
    let mut max_written_tick: u32 = 0;
    let mut server_tick_base: Option<i64> = None;

    let mut progress = 0u32;
    let mut interval_per_tick = 0.0f32;
    let mut world = crate::demo::parser::analyser::World::default();

    loop {
        let packet = match packets.next(handler.get_parser_state()) {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => return JsValue::from_str("Failed to parse demo packet"),
        };

        let packet_tick: u32 = packet.tick().into();

        if handler.handle_packet(packet).is_err() {
            return JsValue::from_str("Failed to parse demo packet");
        }

        let server_tick: u32 = u32::from(handler.server_tick);

        let parser_state = handler.get_parser_state();
        let analyser = handler.analyser();
        let match_state = analyser.match_state(parser_state);
        let game_state = analyser.game_state(parser_state);

        if server_tick_base.is_none() && (match_state.start_tick != 0 || server_tick != 0) {
            server_tick_base = Some(server_tick as i64 - packet_tick as i64);
        }

        if let Some(world_state) = &game_state.world {
            world.boundary_min = world_state.boundary_min;
            world.boundary_max = world_state.boundary_max;
        }

        interval_per_tick = game_state
            .interval_per_tick
            .max(match_state.interval_per_tick);

        if !started {
            if !game_state.players.is_empty() {
                warmup_frames += 1;
            }
            let world_ready = !(world.boundary_min.x == 0.0
                && world.boundary_min.y == 0.0
                && world.boundary_min.z == 0.0
                && world.boundary_max.x == 0.0
                && world.boundary_max.y == 0.0
                && world.boundary_max.z == 0.0);
            if warmup_frames >= 100 && world_ready {
                started = true;
                start_server_tick = server_tick;
                start_demo_tick = packet_tick;
                start_tick_base = server_tick as i64 - packet_tick as i64;
                cache_builder.position_offset = world.boundary_min;
            } else {
                continue;
            }
        }

        // Track how demo ticks map onto server ticks during the cached window.
        let tick_base_current = server_tick as i64 - packet_tick as i64;
        if let Some(prev) = tick_base_last {
            if prev != tick_base_current {
                tick_base_changes += 1;
            }
        }
        tick_base_last = Some(tick_base_current);

        if tick_base_samples == 0 {
            tick_base_min = tick_base_current;
            tick_base_max = tick_base_current;
        } else {
            tick_base_min = tick_base_min.min(tick_base_current);
            tick_base_max = tick_base_max.max(tick_base_current);
        }

        let tick_skew = tick_base_current - start_tick_base;
        if tick_base_samples == 0 {
            tick_skew_min = tick_skew;
            tick_skew_max = tick_skew;
            tick_skew_abs_max = tick_skew.abs();
        } else {
            tick_skew_min = tick_skew_min.min(tick_skew);
            tick_skew_max = tick_skew_max.max(tick_skew);
            tick_skew_abs_max = tick_skew_abs_max.max(tick_skew.abs());
        }

        tick_base_samples += 1;

        if server_tick < start_server_tick {
            continue;
        }
        let internal_tick = server_tick - start_server_tick;
        if internal_tick as usize >= tick_capacity {
            continue;
        }

        if let Some(prev_tick) = last_snapshot_tick {
            if internal_tick <= prev_tick {
                continue;
            }
        }

        let mut current_players: Vec<Option<PlayerSample>> = vec![None; cache_builder.player_count];
        for player in game_state.players.iter() {
            let index = match player_ids.get(&player.entity).copied() {
                Some(existing) => existing,
                None => {
                    let next = player_ids.len();
                    player_ids.insert(player.entity, next);
                    next
                }
            };

            cache_builder.ensure_player(index);
            if current_players.len() < cache_builder.player_count {
                current_players.resize(cache_builder.player_count, None);
            }

            if let Some(info) = player.info.as_ref() {
                user_info_by_entity.insert(player.entity, info.clone());
            }

            let health_value = if player.state == PlayerState::Alive {
                player.health
            } else {
                0
            };

            let (uber, heal_target) =
                if let PlayerClassData::Medic { charge, target, .. } = player.class_data {
                    (
                        charge as u16,
                        target
                            .map(|target| u32::from(target) as u16)
                            .unwrap_or_default(),
                    )
                } else {
                    (0, 0)
                };

            current_players[index] = Some(PlayerSample {
                connected: player.connected,
                position: player.position,
                view_angle: player.view_angle,
                pitch_angle: player.pitch_angle,
                health: health_value,
                class_id: player.class as u8,
                team_id: map_team(player.team),
                uber,
                heal_target,
                flags: player.flags,
            });
        }

        let mut current_projectiles: Vec<Option<ProjectileSample>> =
            vec![None; cache_builder.projectiles.len()];
        for (entity_id, projectile) in game_state.projectiles.iter() {
            let index = cache_builder.ensure_projectile(*entity_id);
            if current_projectiles.len() < cache_builder.projectiles.len() {
                current_projectiles.resize(cache_builder.projectiles.len(), None);
            }
            current_projectiles[index] = Some(ProjectileSample {
                position: projectile.position,
                rotation: projectile.rotation,
                team_id: map_team(projectile.team),
                ty: map_projectile_type(projectile.ty),
            });
        }

        if last_players.len() < cache_builder.player_count {
            last_players.resize(cache_builder.player_count, None);
        }
        if last_projectiles.len() < cache_builder.projectiles.len() {
            last_projectiles.resize(cache_builder.projectiles.len(), None);
        }

        let write_player_tick =
            |builder: &mut CacheBuilder, tick: usize, index: usize, sample: PlayerSample| {
                CacheBuilder::set_vec(
                    &mut builder.positions[index],
                    tick,
                    sample.position,
                    Some(builder.position_offset),
                    POSITION_FIXED_SCALE,
                );
                let view = crate::demo::vector::Vector {
                    x: sample.pitch_angle,
                    y: sample.view_angle,
                    z: 0.0,
                };
                CacheBuilder::set_vec(
                    &mut builder.view_angles[index],
                    tick,
                    view,
                    None,
                    ANGLE_FIXED_SCALE,
                );
                CacheBuilder::set_u16(&mut builder.health[index], tick, sample.health, 2);
                CacheBuilder::set_sparse_u8(
                    &mut builder.meta[index],
                    tick,
                    pack_meta(sample.class_id, sample.team_id),
                    6,
                );
                CacheBuilder::set_sparse_u8(
                    &mut builder.connected[index],
                    tick,
                    if sample.connected { 1 } else { 0 },
                    4,
                );
                CacheBuilder::set_sparse_u16(&mut builder.uber[index], tick, sample.uber, 4);
                CacheBuilder::set_sparse_u16(
                    &mut builder.heal_target[index],
                    tick,
                    sample.heal_target,
                    4,
                );

                let duck = (sample.flags & FL_DUCKING) != 0;
                let anim_duck = (sample.flags & FL_ANIMDUCKING) != 0;
                let duck_state: u8 = (duck as u8) | ((anim_duck as u8) << 1);
                CacheBuilder::set_sparse_u8(&mut builder.duck[index], tick, duck_state, 0);
            };

        let write_projectile_tick = |builder: &mut CacheBuilder,
                                     tick: usize,
                                     index: usize,
                                     sample: ProjectileSample| {
            CacheBuilder::set_vec(
                &mut builder.projectile_positions[index],
                tick,
                sample.position,
                Some(builder.position_offset),
                POSITION_FIXED_SCALE,
            );
            CacheBuilder::set_vec(
                &mut builder.projectile_rotations[index],
                tick,
                sample.rotation,
                None,
                ANGLE_FIXED_SCALE,
            );
            CacheBuilder::set_sparse_u8(
                &mut builder.projectile_team[index],
                tick,
                sample.team_id,
                6,
            );
            CacheBuilder::set_sparse_u8(&mut builder.projectile_type[index], tick, sample.ty, 6);
        };

        match last_snapshot_tick {
            None => {
                let tick = internal_tick as usize;
                for (index, sample) in current_players.iter().enumerate() {
                    if let Some(sample) = sample {
                        if sample.connected {
                            write_player_tick(&mut cache_builder, tick, index, *sample);
                        }
                    }
                }
                for (index, sample) in current_projectiles.iter().enumerate() {
                    if let Some(sample) = sample {
                        write_projectile_tick(&mut cache_builder, tick, index, *sample);
                    }
                }
                last_snapshot_tick = Some(internal_tick);
                last_players = current_players;
                last_projectiles = current_projectiles;
                max_written_tick = internal_tick;
            }
            Some(prev_tick) => {
                let gap = internal_tick - prev_tick;
                let gap_f = gap as f32;

                for step in 1..=gap {
                    let tick = (prev_tick + step) as usize;
                    if tick >= tick_capacity {
                        break;
                    }
                    let alpha = step as f32 / gap_f;
                    let is_final = step == gap;

                    for index in 0..cache_builder.player_count {
                        let prev = last_players.get(index).copied().flatten();
                        let cur = current_players.get(index).copied().flatten();

                        let prev_connected = prev.map(|v| v.connected).unwrap_or(false);
                        let cur_connected = cur.map(|v| v.connected).unwrap_or(false);

                        let sample = if prev_connected && cur_connected {
                            let prev = prev.unwrap_or_default();
                            let cur = cur.unwrap_or_default();
                            let teleport =
                                distance(prev.position, cur.position) > TELEPORT_DISTANCE;
                            if teleport {
                                if is_final {
                                    Some(cur)
                                } else {
                                    Some(prev)
                                }
                            } else {
                                let position = lerp_vec(prev.position, cur.position, alpha);
                                let view_angle =
                                    lerp_angle_deg(prev.view_angle, cur.view_angle, alpha);
                                let pitch_angle =
                                    lerp_angle_deg(prev.pitch_angle, cur.pitch_angle, alpha);
                                Some(PlayerSample {
                                    connected: true,
                                    position,
                                    view_angle,
                                    pitch_angle,
                                    health: if is_final { cur.health } else { prev.health },
                                    class_id: if is_final {
                                        cur.class_id
                                    } else {
                                        prev.class_id
                                    },
                                    team_id: if is_final { cur.team_id } else { prev.team_id },
                                    uber: if is_final { cur.uber } else { prev.uber },
                                    heal_target: if is_final {
                                        cur.heal_target
                                    } else {
                                        prev.heal_target
                                    },
                                    flags: if is_final { cur.flags } else { prev.flags },
                                })
                            }
                        } else if prev_connected && !cur_connected {
                            if is_final {
                                None
                            } else {
                                prev
                            }
                        } else if !prev_connected && cur_connected {
                            if is_final {
                                cur
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if let Some(sample) = sample {
                            if sample.connected {
                                write_player_tick(&mut cache_builder, tick, index, sample);
                            }
                        }
                    }

                    for index in 0..cache_builder.projectiles.len() {
                        let prev = last_projectiles.get(index).copied().flatten();
                        let cur = current_projectiles.get(index).copied().flatten();
                        let sample = match (prev, cur) {
                            (Some(prev), Some(cur)) => {
                                let teleport =
                                    distance(prev.position, cur.position) > TELEPORT_DISTANCE;
                                if teleport {
                                    if is_final {
                                        Some(cur)
                                    } else {
                                        Some(prev)
                                    }
                                } else {
                                    Some(ProjectileSample {
                                        position: lerp_vec(prev.position, cur.position, alpha),
                                        rotation: lerp_vec(prev.rotation, cur.rotation, alpha),
                                        team_id: if is_final { cur.team_id } else { prev.team_id },
                                        ty: if is_final { cur.ty } else { prev.ty },
                                    })
                                }
                            }
                            (Some(prev), None) => {
                                if is_final {
                                    None
                                } else {
                                    Some(prev)
                                }
                            }
                            (None, Some(cur)) => {
                                if is_final {
                                    Some(cur)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };

                        if let Some(sample) = sample {
                            write_projectile_tick(&mut cache_builder, tick, index, sample);
                        }
                    }

                    max_written_tick = max_written_tick.max((prev_tick + step));
                }

                last_snapshot_tick = Some(internal_tick);
                last_players = current_players;
                last_projectiles = current_projectiles;
            }
        }

        if let Some(callback) = progress_callback {
            let next_progress = ((internal_tick as f32 / tick_capacity as f32) * 100.0) as u32;
            if next_progress > progress && next_progress <= 100 {
                progress = next_progress;
                let _ = callback.call1(&JsValue::NULL, &JsValue::from_f64(progress as f64));
            }
        }
    }

    if let Some(callback) = progress_callback {
        if progress < 100 {
            let _ = callback.call1(&JsValue::NULL, &JsValue::from_f64(100.0));
        }
    }

    let output_state = handler.into_output();
    let users_by_id: BTreeMap<_, _> = output_state.match_state.users.clone();
    let output_ticks = ((max_written_tick + 1) as usize).max(1);
    let tick_base_first = server_tick_base.unwrap_or(start_tick_base);
    let tick_base = if started {
        start_tick_base
    } else {
        tick_base_first
    };
    let mut deaths_by_tick: BTreeMap<u32, Vec<CachedDeathEntry>> = BTreeMap::new();

    for death in output_state.match_state.deaths.iter() {
        let raw_tick = u32::from(death.tick) as i64;
        let server_tick = tick_base + raw_tick;
        if server_tick < start_server_tick as i64 {
            continue;
        }

        let internal_tick = (server_tick - start_server_tick as i64) as u32;
        if internal_tick as usize >= output_ticks {
            continue;
        }

        let victim = users_by_id.get(&death.victim).cloned();
        let killer = users_by_id.get(&death.killer).cloned();
        let assister = death
            .assister
            .and_then(|assister| users_by_id.get(&assister).cloned());

        let Some(victim) = victim else {
            continue;
        };

        let entry = CachedDeathEntry {
            tick: internal_tick,
            victim: PlayerRef {
                user: victim.clone(),
            },
            assister: assister.clone().map(|user| PlayerRef { user }),
            killer: killer.clone().map(|user| PlayerRef { user }),
            weapon: death.weapon.clone(),
            victim_team: match_team_to_id(victim.team),
            assister_team: assister
                .as_ref()
                .map(|user| match_team_to_id(user.team))
                .unwrap_or_default(),
            killer_team: killer
                .as_ref()
                .map(|user| match_team_to_id(user.team))
                .unwrap_or_default(),
        };

        deaths_by_tick.entry(internal_tick).or_default().push(entry);
    }

    let rounds: Vec<RoundEntry> = output_state
        .match_state
        .rounds
        .iter()
        .filter_map(|round| {
            let raw_tick = u32::from(round.end_tick) as i64;
            let server_tick = tick_base + raw_tick;
            if server_tick < start_server_tick as i64 {
                return None;
            }

            let internal_tick = (server_tick - start_server_tick as i64) as u32;
            if internal_tick as usize >= output_ticks {
                return None;
            }
            Some(RoundEntry {
                winner: match_team_to_str(round.winner).to_string(),
                length: round.length,
                end_tick: internal_tick,
            })
        })
        .collect();

    let chat: Vec<ChatEntry> = output_state
        .match_state
        .chat
        .iter()
        .filter_map(|msg| {
            let raw_tick = u32::from(msg.tick) as i64;
            let server_tick = tick_base + raw_tick;
            if server_tick < start_server_tick as i64 {
                return None;
            }

            let internal_tick = (server_tick - start_server_tick as i64) as u32;
            if internal_tick as usize >= output_ticks {
                return None;
            }

            let client_entity_id = msg.client.map(|id| u32::from(id));
            let client_player_id = msg
                .client
                .and_then(|id| player_ids.get(&id).copied())
                .map(|id| id as u32);

            let raw_text = if msg.from.is_empty() {
                msg.text.clone()
            } else {
                match msg.kind {
                    crate::demo::message::usermessage::ChatMessageKind::ChatTeam => {
                        format!("\x01(TEAM) \x03{}\x01: {}", msg.from, msg.text)
                    }
                    crate::demo::message::usermessage::ChatMessageKind::ChatAllDead => {
                        format!("\x01*DEAD* \x03{}\x01: {}", msg.from, msg.text)
                    }
                    crate::demo::message::usermessage::ChatMessageKind::ChatTeamDead => {
                        format!("\x01*DEAD* (TEAM) \x03{}\x01: {}", msg.from, msg.text)
                    }
                    crate::demo::message::usermessage::ChatMessageKind::ChatAllSpec => {
                        format!("\x01*SPEC* \x03{}\x01: {}", msg.from, msg.text)
                    }
                    _ => format!("\x03{}\x01: {}", msg.from, msg.text),
                }
            };

            Some(ChatEntry {
                tick: internal_tick,
                kind: msg.kind,
                client_entity_id,
                client_player_id,
                raw_text,
            })
        })
        .collect();

    let header_js = match to_value(&header) {
        Ok(value) => value,
        Err(_) => return JsValue::from_str("Failed to serialize header"),
    };
    let result = Object::new();
    let _ = Reflect::set(&result, &JsValue::from_str("header"), &header_js);
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("ticks"),
        &JsValue::from_f64(output_ticks as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("startTick"),
        &JsValue::from_f64(start_server_tick as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("startDemoTick"),
        &JsValue::from_f64(start_demo_tick as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("tickBase"),
        &JsValue::from_f64(tick_base as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("tickBaseFirst"),
        &JsValue::from_f64(tick_base_first as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("tickBaseSamples"),
        &JsValue::from_f64(tick_base_samples as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("tickBaseChanges"),
        &JsValue::from_f64(tick_base_changes as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("tickBaseMin"),
        &JsValue::from_f64(tick_base_min as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("tickBaseMax"),
        &JsValue::from_f64(tick_base_max as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("tickSkewMin"),
        &JsValue::from_f64(tick_skew_min as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("tickSkewMax"),
        &JsValue::from_f64(tick_skew_max as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("tickSkewAbsMax"),
        &JsValue::from_f64(tick_skew_abs_max as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("intervalPerTick"),
        &JsValue::from_f64(interval_per_tick as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("positionScale"),
        &JsValue::from_f64(POSITION_FIXED_SCALE as f64),
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("angleScale"),
        &JsValue::from_f64(ANGLE_FIXED_SCALE as f64),
    );
    let world_js = JsWorld {
        boundary_min: world.boundary_min,
        boundary_max: world.boundary_max,
    };
    let world_js = match to_value(&world_js) {
        Ok(value) => value,
        Err(_) => return JsValue::from_str("Failed to serialize world"),
    };
    let _ = Reflect::set(&result, &JsValue::from_str("world"), &world_js);

    let users_by_entity: BTreeMap<_, _> = users_by_id
        .values()
        .map(|user| (user.entity_id, user.clone()))
        .collect();

    let players = Array::new();
    for (entity_id, index) in player_ids.iter() {
        let user = users_by_entity
            .get(entity_id)
            .cloned()
            .or_else(|| user_info_by_entity.get(entity_id).cloned());
        if let Some(user) = user {
            let entry = Object::new();
            let _ = Reflect::set(
                &entry,
                &JsValue::from_str("id"),
                &JsValue::from_f64(*index as f64),
            );
            let _ = Reflect::set(
                &entry,
                &JsValue::from_str("entityId"),
                &JsValue::from_f64(u32::from(*entity_id) as f64),
            );
            let _ = Reflect::set(
                &entry,
                &JsValue::from_str("userId"),
                &JsValue::from_f64(u32::from(user.user_id) as f64),
            );
            let _ = Reflect::set(
                &entry,
                &JsValue::from_str("steamId"),
                &JsValue::from_str(&user.steam_id),
            );
            let _ = Reflect::set(
                &entry,
                &JsValue::from_str("name"),
                &JsValue::from_str(&user.name),
            );
            let _ = Reflect::set(
                &entry,
                &JsValue::from_str("team"),
                &JsValue::from_str(match_team_to_str(user.team)),
            );
            players.push(&entry);
        }
    }

    let _ = Reflect::set(&result, &JsValue::from_str("players"), &players);

    let player_cache = Object::new();
    let position_list = Array::new();
    for cache in cache_builder.positions.iter() {
        position_list.push(&Uint32Array::from(cache.as_slice()));
    }
    let view_list = Array::new();
    for cache in cache_builder.view_angles.iter() {
        view_list.push(&Uint32Array::from(cache.as_slice()));
    }
    let health_list = Array::new();
    for cache in cache_builder.health.iter() {
        health_list.push(&Uint16Array::from(cache.as_slice()));
    }
    let meta_list = Array::new();
    for cache in cache_builder.meta.iter() {
        meta_list.push(&Uint8Array::from(cache.as_slice()));
    }
    let connected_list = Array::new();
    for cache in cache_builder.connected.iter() {
        connected_list.push(&Uint8Array::from(cache.as_slice()));
    }
    let uber_list = Array::new();
    for cache in cache_builder.uber.iter() {
        uber_list.push(&Uint16Array::from(cache.as_slice()));
    }
    let heal_list = Array::new();
    for cache in cache_builder.heal_target.iter() {
        heal_list.push(&Uint16Array::from(cache.as_slice()));
    }
    let duck_list = Array::new();
    for cache in cache_builder.duck.iter() {
        duck_list.push(&Uint8Array::from(cache.as_slice()));
    }

    let _ = Reflect::set(
        &player_cache,
        &JsValue::from_str("position"),
        &position_list,
    );
    let _ = Reflect::set(&player_cache, &JsValue::from_str("viewAngles"), &view_list);
    let _ = Reflect::set(&player_cache, &JsValue::from_str("health"), &health_list);
    let _ = Reflect::set(&player_cache, &JsValue::from_str("meta"), &meta_list);
    let _ = Reflect::set(
        &player_cache,
        &JsValue::from_str("connected"),
        &connected_list,
    );
    let _ = Reflect::set(&player_cache, &JsValue::from_str("uber"), &uber_list);
    let _ = Reflect::set(&player_cache, &JsValue::from_str("healTarget"), &heal_list);
    let _ = Reflect::set(&player_cache, &JsValue::from_str("duck"), &duck_list);
    let _ = Reflect::set(&result, &JsValue::from_str("playerCache"), &player_cache);

    let projectile_cache = Object::new();
    let proj_position_list = Array::new();
    let proj_rotation_list = Array::new();
    let proj_team_list = Array::new();
    let proj_type_list = Array::new();
    let mut projectile_ids = vec![0u32; cache_builder.projectiles.len()];
    for (entity_id, index) in cache_builder.projectiles.iter() {
        projectile_ids[*index] = u32::from(*entity_id);
    }
    let proj_id_list = Array::new();
    for id in projectile_ids.iter() {
        proj_id_list.push(&JsValue::from_f64(*id as f64));
    }
    for cache in cache_builder.projectile_positions.iter() {
        proj_position_list.push(&Uint32Array::from(cache.as_slice()));
    }
    for cache in cache_builder.projectile_rotations.iter() {
        proj_rotation_list.push(&Uint32Array::from(cache.as_slice()));
    }
    for cache in cache_builder.projectile_team.iter() {
        proj_team_list.push(&Uint8Array::from(cache.as_slice()));
    }
    for cache in cache_builder.projectile_type.iter() {
        proj_type_list.push(&Uint8Array::from(cache.as_slice()));
    }
    let _ = Reflect::set(&projectile_cache, &JsValue::from_str("ids"), &proj_id_list);
    let _ = Reflect::set(
        &projectile_cache,
        &JsValue::from_str("position"),
        &proj_position_list,
    );
    let _ = Reflect::set(
        &projectile_cache,
        &JsValue::from_str("rotation"),
        &proj_rotation_list,
    );
    let _ = Reflect::set(
        &projectile_cache,
        &JsValue::from_str("team"),
        &proj_team_list,
    );
    let _ = Reflect::set(
        &projectile_cache,
        &JsValue::from_str("type"),
        &proj_type_list,
    );
    let _ = Reflect::set(
        &result,
        &JsValue::from_str("projectileCache"),
        &projectile_cache,
    );

    let deaths_js = Object::new();
    for (tick, deaths) in deaths_by_tick.iter() {
        let deaths_value = match to_value(deaths) {
            Ok(value) => value,
            Err(_) => return JsValue::from_str("Failed to serialize deaths"),
        };
        let _ = Reflect::set(
            &deaths_js,
            &JsValue::from_str(&tick.to_string()),
            &deaths_value,
        );
    }
    let _ = Reflect::set(&result, &JsValue::from_str("deaths"), &deaths_js);
    let rounds_js = match to_value(&rounds) {
        Ok(value) => value,
        Err(_) => return JsValue::from_str("Failed to serialize rounds"),
    };
    let _ = Reflect::set(&result, &JsValue::from_str("rounds"), &rounds_js);

    let chat_js = match to_value(&chat) {
        Ok(value) => value,
        Err(_) => return JsValue::from_str("Failed to serialize chat"),
    };
    let _ = Reflect::set(&result, &JsValue::from_str("chat"), &chat_js);

    JsValue::from(result)
}

#[wasm_bindgen]
pub fn parse_demo_cache(buffer: &[u8]) -> JsValue {
    parse_demo_cache_internal(buffer, None)
}

#[wasm_bindgen]
pub fn parse_demo_cache_with_progress(
    buffer: &[u8],
    progress_callback: js_sys::Function,
) -> JsValue {
    parse_demo_cache_internal(buffer, Some(&progress_callback))
}

#[derive(Default)]
struct CacheBuilder {
    tick_count: usize,
    position_offset: crate::demo::vector::Vector,
    player_count: usize,
    positions: Vec<Vec<u32>>,
    view_angles: Vec<Vec<u32>>,
    health: Vec<Vec<u16>>,
    meta: Vec<Vec<u8>>,
    connected: Vec<Vec<u8>>,
    uber: Vec<Vec<u16>>,
    heal_target: Vec<Vec<u16>>,
    duck: Vec<Vec<u8>>,
    projectiles: BTreeMap<EntityId, usize>,
    projectile_positions: Vec<Vec<u32>>,
    projectile_rotations: Vec<Vec<u32>>,
    projectile_team: Vec<Vec<u8>>,
    projectile_type: Vec<Vec<u8>>,
}

impl CacheBuilder {
    fn new(tick_count: usize, position_offset: crate::demo::vector::Vector) -> Self {
        CacheBuilder {
            tick_count,
            position_offset,
            ..CacheBuilder::default()
        }
    }

    fn ensure_player(&mut self, player_index: usize) {
        if player_index < self.player_count {
            return;
        }
        while self.player_count <= player_index {
            let position_len = self.tick_count * 3;
            let health_len = (self.tick_count >> 2).max(1);
            let meta_len = (self.tick_count >> 6).max(1);
            let connected_len = (self.tick_count >> 4).max(1);
            let duck_len = self.tick_count;
            self.positions.push(vec![0; position_len]);
            self.view_angles.push(vec![0; position_len]);
            self.health.push(vec![0; health_len]);
            self.meta.push(vec![0; meta_len]);
            self.connected.push(vec![0; connected_len]);
            self.uber.push(vec![0; connected_len]);
            self.heal_target.push(vec![0; connected_len]);
            self.duck.push(vec![0; duck_len]);
            self.player_count += 1;
        }
    }

    fn ensure_projectile(&mut self, entity_id: EntityId) -> usize {
        if let Some(index) = self.projectiles.get(&entity_id) {
            return *index;
        }
        let index = self.projectiles.len();
        self.projectiles.insert(entity_id, index);
        let position_len = self.tick_count * 3;
        let sparse_len = (self.tick_count >> 6).max(1);
        self.projectile_positions.push(vec![0; position_len]);
        self.projectile_rotations.push(vec![0; position_len]);
        self.projectile_team.push(vec![0; sparse_len]);
        self.projectile_type.push(vec![0; sparse_len]);
        index
    }

    fn set_vec(
        target: &mut [u32],
        tick: usize,
        vector: crate::demo::vector::Vector,
        offset: Option<crate::demo::vector::Vector>,
        quantize_scale: f32,
    ) {
        let base = tick * 3;
        let (x, y, z) = (vector.x, vector.y, vector.z);
        let (x, y, z) = if let Some(offset) = offset {
            (x - offset.x, y - offset.y, z - offset.z)
        } else {
            (x, y, z)
        };
        target[base + 0] = to_fixed_u32(x, quantize_scale);
        target[base + 1] = to_fixed_u32(y, quantize_scale);
        target[base + 2] = to_fixed_u32(z, quantize_scale);
    }

    fn set_u16(target: &mut [u16], tick: usize, value: u16, shift: u8) {
        let index = tick >> shift;
        if index < target.len() {
            target[index] = value;
        }
    }

    fn set_sparse_u8(target: &mut [u8], tick: usize, value: u8, shift: u8) {
        let index = tick >> shift;
        if index < target.len() {
            target[index] = value;
        }
    }

    fn set_sparse_u16(target: &mut [u16], tick: usize, value: u16, shift: u8) {
        let index = tick >> shift;
        if index < target.len() {
            target[index] = value;
        }
    }
}
