# formdata

[![Build Status](https://travis-ci.org/mikedilger/formdata.svg?branch=master)](https://travis-ci.org/mikedilger/formdata)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![crates.io](http://meritbadge.herokuapp.com/formdata)](https://crates.io/crates/formdata)

Documentation is available at https://mikedilger.github.io/formdata

This library provides a function for parsing a stream in `multipart/form-data`
format. It separates embedded files and streams them to disk.

HTML forms with enctype=`multipart/form-data` `POST` their data in this
format. This `enctype` is used whenever a form has file upload input fields,
as the default `application/x-www-form-urlencoded` cannot handle file
uploads.

## Example

```rust
// request is your `hyper::server::Request` or any type that implements
// the `formdata::Request` trait

let form_data = try!(parse_multipart(&mut request));

for (name, value) in form_data.fields {
    println!("Posted field name={} value={}",name,value);
}

for (name, file) in form_data.files {
    println!("Posted file name={} filename={} content_type={} size={} temporary_path={}",
             name, file.filename, file.content_type, file.size, file.path);
}

```
