// Copyright © 2015 by Michael Dilger (of New Zealand)
// This code is licensed under the MIT license (see LICENSE-MIT for details)

//! This crate parses and processes a stream of data that contains
//! `multipart/form-data` content.
//!
//! The main entry point is `parse_multipart`

#![cfg_attr(not(feature = "with-syntex"), feature(custom_attribute, custom_derive, plugin))]
#![cfg_attr(not(feature = "with-syntex"), plugin(serde_macros))]

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

#[cfg(feature = "with-syntex")]
include!(concat!(env!("OUT_DIR"), "/lib.rs"));

#[cfg(not(feature = "with-syntex"))]
include!("lib.rs.in");
