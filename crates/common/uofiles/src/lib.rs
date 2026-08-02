//! Readers for the client's own files: the map, tiledata, the palettes, the
//! art, and the UOP container format they can be shipped in. Below the server
//! so a renderer on the client side can read the same files without depending
//! on `server/*`.
//!
//! What the shard needs and what a renderer needs part company here: a server
//! reads [`map`] and [`tiledata`] and never opens the art, while a renderer
//! needs [`art`], [`texmaps`] and [`hues`] and cares about [`color`]. They share
//! a crate because they share files — the same `tiledata` that says a tile
//! blocks a step says which art to draw for it, and which texture to stretch
//! over it where the ground slopes.
//!
//! No client files ever enter this repository. Tests that need real data read
//! `OPENSHARD_CLIENT` and skip when it is unset; `tests/client_files.rs` is
//! where the readers are held against a shipped install rather than against a
//! fixture they agree with by construction.

pub mod anim;
pub mod animdata;
pub mod art;
pub mod color;
pub mod equipconv;
pub mod font;
pub mod gumpart;
pub mod hues;
pub mod image;
pub mod map;
pub mod texmaps;
pub mod tiledata;
pub mod uop;
