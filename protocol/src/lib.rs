//! The display protocol described in `PROTOCOL.md`, both ends of it: what the PC app writes
//! the display's firmware reads here, and the other way round.
//!
//! Only `core` and `heapless`, so the firmware can use it.

#![cfg_attr(not(test), no_std)]

pub mod board;
pub mod frame;
pub mod udp;

pub use frame::{Frame, Match, OwnStats, Parser, Player, Reply};
