//! This crate parses and processes hyper::server::Request data
//! containing multipart/form-data formatted data in a streaming
//! fashion.

extern crate hyper;

#[cfg(test)]
mod mock;
