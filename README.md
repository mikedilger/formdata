# formdata

[![Build Status](https://travis-ci.org/mikedilger/formdata.svg?branch=master)](https://travis-ci.org/mikedilger/formdata)

Documentation is available at https://mikedilger.github.io/formdata

This library provides a function for parsing a stream in `multipart/form-data`
format, such as what HTTP user agents (browsers) send to HTTP servers via the
POST method in response to HTTP forms with enctype="multipart/form-data".
These streams typically contain embedded uploaded files.  This library
separates those files and streams them to disk.

## Example

```rust
// request is your ::hyper::server::Request

if let Ok((fields,files)) = parse_multipart(&mut request) {
    for (name,value) in fields {
        println!("Posted variable name={} value={}",name,value);
    }
    for (name,file) in files {
        println!("Posted file name={} filename={} content_type={} size={}
                  temporary_path={}",
                 name, file.filename, file.content_type, file.size, file.path);
    }
}

```
