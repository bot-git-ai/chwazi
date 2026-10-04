// Copyright (c) 2026 Witalis Domitrz <witekdomitrz@gmail.com>
// AGPL License

//! Chwazi, the finger chooser: the rules in Rust, the pixels in Rust, and a
//! static HTML file that does nothing but load the wasm.
//!
//! The app is a phone passed around a table: every finger on the glass becomes
//! a coloured circle, and after a couple of seconds of stillness one of them is
//! chosen at random and its colour takes the screen. [`chooser`] is all of that
//! behaviour, with no browser and no clock in it, so it can be tested
//! exhaustively; `ui` is the thin web-sys layer that feeds it pointer events and
//! draws what it decides. It is a module rather than a link because it only
//! exists on `wasm32` -- it is the browser layer -- and a link to it would break
//! documentation for the host build, which is the one the tests run on.
//!
//! There is no binary and no server: `cargo build` writes `dist/`, and that
//! directory is the whole publishable artifact.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

pub mod chooser;

#[cfg(target_arch = "wasm32")]
mod ui;

pub use chooser::{Chooser, Player};
