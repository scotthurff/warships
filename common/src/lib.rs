// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

#![feature(array_chunks)]
#![feature(test)]
#![feature(let_chains)]

use kodiak_common::{DefaultedGameConstants, GameConstants};

// Actually required see https://doc.rust-lang.org/beta/unstable-book/library-features/test.html
#[cfg(test)]
extern crate core;
#[cfg(test)]
extern crate test;

pub const MK48_CONSTANTS: &'static GameConstants = &GameConstants {
    game_id: "Warships",
    name: "WARSHIPS",
    // Kodiak's check_origin (router.rs) only allows WebSocket upgrades
    // from origins ending in this domain, "softbear.com", or
    // localhost/127.0.0.1 / port 8443. When deployed behind Fly.io
    // the browser sends Origin: https://warships.fly.dev and was
    // rejected with 401 "invalid origin", breaking every client
    // WebSocket. Pointing this at the live host unblocks prod; local
    // dev still works via the separate localhost allow-branch.
    domain: "warships.fly.dev",
    geodns_enabled: false,
    trademark: "WARSHIPS",
    server_names: &[
        "Atlantic", "Pacific", "Fjord", "Kraken", "Scotia", "Barents", "Bering", "Chukchi",
    ],
    defaulted: DefaultedGameConstants::new(),
};

/// Game difficulty level — affects bot AI behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Difficulty {
    #[default]
    Captain,    // Easy — for young kids
    Admiral,    // Medium — original mk48 defaults
    FleetCommander, // Hard — aggressive bots
}

/// Global difficulty setting. Atomic so server can read it.
use std::sync::atomic::{AtomicU8, Ordering};
static GLOBAL_DIFFICULTY: AtomicU8 = AtomicU8::new(0);

impl Difficulty {
    pub fn set_global(d: Difficulty) {
        GLOBAL_DIFFICULTY.store(d as u8, Ordering::Relaxed);
    }

    pub fn get_global() -> Difficulty {
        match GLOBAL_DIFFICULTY.load(Ordering::Relaxed) {
            1 => Difficulty::Admiral,
            2 => Difficulty::FleetCommander,
            _ => Difficulty::Captain,
        }
    }
}

pub mod altitude;
pub mod angle;
pub mod complete;
pub mod contact;
pub mod death_reason;
pub mod entity;
pub mod guidance;
pub mod protocol;
pub mod terrain;
pub mod ticks;
pub mod transform;
pub mod util;
pub mod velocity;
pub mod world;
