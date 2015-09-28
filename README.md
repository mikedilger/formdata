# formdata

[![Build Status](https://travis-ci.org/mikedilger/formdata.svg?branch=master)](https://travis-ci.org/mikedilger/formdata)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

Documentation is available at https://mikedilger.github.io/formdata

This library provides a function for server-side parsing of a stream in
`multipart/form-data` format as described by [RFC 7578](https://tools.ietf.org/html/rfc7578).
It separates embedded files and streams them to temporary files on disk, while collecting
the other form fields into the returned data structure.

HTML forms with enctype=`multipart/form-data` `POST` their data in this format.
This `enctype` is typically used whenever a form has file upload input fields,
as the default `application/x-www-form-urlencoded` cannot handle file uploads.

## Example

```rust
// `headers` is your `hyper::headers::Headers` from your hyper or iron request.
// `request` is the readable stream, and could be your hyper Request or iron HttpRequest, or

let boundary = try!(get_multipart_boundary(&headers));
let form_data = try!(parse_multipart(&mut request, boundary));

for (name, value) in form_data.fields {
    println!("Posted field name={} value={}",name,value);
}

for (name, file) in form_data.files {
    println!("Posted file name={} filename={:?} content_type={} size={} temporary_path={:?}",
             name, file.filename, file.content_type, file.size, file.path);
}

```
