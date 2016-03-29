// Copyright © 2015 by Michael Dilger (of New Zealand)
// This code is licensed under the MIT license (see LICENSE-MIT for details)

//! This crate parses and processes a stream of data that contains
//! `multipart/form-data` content.
//!
//! The main entry point is `parse_multipart`

#![cfg_attr(feature = "rust-nightly", feature(custom_attribute, custom_derive, plugin))]
#![cfg_attr(feature = "rust-nightly", plugin(serde_macros))]
#![cfg_attr(feature="clippy", feature(plugin))]
#![cfg_attr(feature="clippy", plugin(clippy))]

extern crate httparse;
extern crate hyper;
#[macro_use]
extern crate mime;
extern crate tempdir;
extern crate textnonce;
#[macro_use]
extern crate log;
extern crate serde;
#[cfg(test)]
extern crate serde_json;
extern crate encoding;

#[cfg(feature = "rust-stable")]
include!(concat!(env!("OUT_DIR"), "/lib.rs"));

#[cfg(feature = "rust-nightly")]
include!("lib.rs.in");
