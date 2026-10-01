//! Wallet logic with no I/O: no database, no network. It compiles to WebAssembly, so the
//! browser wallet runs exactly the same key handling and PSBT checks as the CLI.

pub mod keys;
pub mod review;
pub mod sign;
