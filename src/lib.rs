//! This crate parses and processes hyper::server::Request data
//! containing multipart/form-data formatted data in a streaming
//! fashion.

#![feature(buf_stream,collections)]

extern crate hyper;

#[cfg(test)]
mod mock;
pub mod buf;

pub use buf::BufReadPlus;
