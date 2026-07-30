//! Readers for the client's own files: the map, tiledata, and the UOP
//! container format they can be shipped in. Below the server so a renderer on
//! the client side can read the same files without depending on `server/*`.
//!
//! No client files ever enter this repository. Tests that need real data read
//! `OPENSHARD_CLIENT` and skip when it is unset.

pub mod map;
pub mod tiledata;
pub mod uop;
