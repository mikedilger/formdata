// Copyright © 2015 by Michael Dilger (of New Zealand)
// This code is licensed under the MIT license (see LICENSE-MIT for details)

//! This library provides a type for storing `multipart/form-data` data, as well as functions
//! to stream (read or write) such data over HTTP.
//!
//! `multipart/form-data` format as described by [RFC 7578](https://tools.ietf.org/html/rfc7578).
//! HTML forms with enctype=`multipart/form-data` `POST` their data in this format.
//! This `enctype` is typically used whenever a form has file upload input fields,
//! as the default `application/x-www-form-urlencoded` cannot handle file uploads.
//!
//! Whether reading from a stream or writing out to a stream, files are never stored entirely
//! in memory, but instead streamed through a buffer.
//!
//! ## Read Example
//!
//! ```no_run
//! extern crate mime;
//! extern crate hyper;
//! extern crate formdata;
//!
//! use hyper::server::{Server, Request, Response};
//!
//! fn main() {
//!   let server = Server::http("0.0.0.0:0").unwrap().handle(handler).unwrap();
//! }
//!
//! fn handler(hyper_request: Request, res: Response) {
//!   let (_, _, headers, _, _, mut reader) = hyper_request.deconstruct();
//!   let form_data = formdata::read_formdata(&mut reader, &headers).unwrap();
//!
//!   for (name, value) in form_data.fields {
//!     println!("Posted field name={} value={}", name, value);
//!   }
//!
//!   for (name, file) in form_data.files {
//!     println!("Posted file name={} path={:?}", name, file.path);
//!   }
//! }
//! ```
//!
//! ## Write Example
//!
//! ```no_run
//! extern crate mime;
//! extern crate hyper;
//! extern crate formdata;
//!
//! use std::path::Path;
//! use hyper::header::{Headers, ContentType};
//! use mime::{Mime, TopLevel, SubLevel};
//! use formdata::{FormData, FilePart};
//!
//! fn main() {
//!   let mut stream = ::std::io::stdout();
//!   let mut photo_headers = Headers::new();
//!   photo_headers.set(ContentType(Mime(TopLevel::Image, SubLevel::Gif, vec![])));
//!   // no need to set Content-Disposition (in fact it will be rewritten)
//!
//!   let formdata = FormData {
//!     fields: vec![ ("name".to_owned(), "Baxter".to_owned()),
//!                   ("age".to_owned(), "1 month".to_owned()) ],
//!     files: vec![ ("photo".to_owned(), FilePart::new(
//!          photo_headers, Path::new("/tmp/puppy.gif"))) ],
//!   };
//!
//!   let boundary = formdata::generate_boundary();
//!   let count = formdata::write_formdata(&mut stream, &boundary, &formdata).unwrap();
//!   println!("COUNT = {}", count);
//! }
//! ```

#![cfg_attr(feature="clippy", feature(plugin))]
#![cfg_attr(feature="clippy", plugin(clippy))]

extern crate httparse;
extern crate hyper;
#[cfg_attr(test, macro_use)]
extern crate mime;
extern crate textnonce;
extern crate log;
extern crate encoding;

extern crate mime_multipart;

mod error;
mod form_data;
#[cfg(test)]
mod mock;

pub use error::Error;
pub use form_data::FormData;

use std::io::{Read, Write};
use hyper::header::{Headers, ContentDisposition, DispositionParam};
use mime_multipart::Node;
pub use mime_multipart::FilePart;
pub use mime_multipart::generate_boundary;

/// Parse MIME `multipart/form-data` information from a stream as a `FormData`.
pub fn read_formdata<S: Read>(stream: &mut S, headers: &Headers) -> Result<FormData, Error>
{
    let nodes = mime_multipart::read_multipart_body(stream, headers, false)?;

    let mut formdata = FormData::new();
    fill_formdata(&mut formdata, nodes)?;
    Ok(formdata)
}

// order and nesting are irrelevant, so we interate through the nodes and put them
// into one of two buckets (fields and files);  If a multipart node is found, it uses
// the name in its headers as the key (rather than the name in the headers of the
// subparts), which is how multiple file uploads work.
fn fill_formdata(formdata: &mut FormData, nodes: Vec<Node>) -> Result<(), Error>
{
    for node in nodes {
        match node {
            Node::Part(part) => {
                let cd_name: Option<String> = {
                    let cd: &ContentDisposition = match part.headers.get() {
                        Some(cd) => cd,
                        None => return Err(Error::MissingDisposition),
                    };
                    get_content_disposition_name(&cd)
                };
                let key = cd_name.ok_or(Error::NoName)?;
                let val = String::from_utf8(part.body)?;
                formdata.fields.push((key, val));
            },
            Node::File(part) => {
                let cd_name: Option<String> = {
                    let cd: &ContentDisposition = match part.headers.get() {
                        Some(cd) => cd,
                        None => return Err(Error::MissingDisposition),
                    };
                    get_content_disposition_name(&cd)
                };
                let key = cd_name.ok_or(Error::NoName)?;
                formdata.files.push((key, part));
            }
            Node::Multipart((headers, nodes)) => {
                let cd_name: Option<String> = {
                    let cd: &ContentDisposition = match headers.get() {
                        Some(cd) => cd,
                        None => return Err(Error::MissingDisposition),
                    };
                    get_content_disposition_name(&cd)
                };
                let key = cd_name.ok_or(Error::NoName)?;
                for node in nodes {
                    match node {
                        Node::Part(part) => {
                            let val = String::from_utf8(part.body)?;
                            formdata.fields.push((key.clone(), val));
                        },
                        Node::File(part) => {
                            formdata.files.push((key.clone(), part));
                        },
                        _ => { } // don't recurse deeper
                    }
                }
            }
        }
    }
    Ok(())
}

#[inline]
fn get_content_disposition_name(cd: &ContentDisposition) -> Option<String> {
    if let Some(&DispositionParam::Ext(_, ref value)) = cd.parameters.iter()
        .find(|&x| match *x {
            DispositionParam::Ext(ref token,_) => &*token == "name",
            _ => false,
        })
    {
        Some(value.clone())
    } else {
        None
    }
}


/// Stream out `multipart/form-data` body content matching the passed in `formdata`.  This
/// does not stream out headers, so the caller must stream those out before calling
/// write_formdata().
pub fn write_formdata<S: Write>(stream: &mut S, boundary: &Vec<u8>, formdata: &FormData)
                                -> Result<usize, Error>
{
    let nodes = formdata.to_multipart()?;

    // Write out
    let count = ::mime_multipart::write_multipart(stream, boundary, &nodes)?;

    Ok(count)
}

/// Stream out `multipart/form-data` body content matching the passed in `formdata` as
/// Transfer-Encoding: Chunked.  This does not stream out headers, so the caller must stream
/// those out before calling write_formdata().
pub fn write_formdata_chunked<S: Write>(stream: &mut S, boundary: &Vec<u8>, formdata: &FormData)
                                        -> Result<(), Error>
{
    let nodes = formdata.to_multipart()?;

    // Write out
    ::mime_multipart::write_multipart_chunked(stream, boundary, &nodes)?;

    Ok(())
}


#[cfg(test)]
mod tests {
    extern crate tempdir;

    use super::{FormData, read_formdata, write_formdata, write_formdata_chunked,
                FilePart, generate_boundary};

    use std::net::SocketAddr;
    use std::fs::File;
    use std::io::Write;

    use hyper::buffer::BufReader;
    use hyper::net::NetworkStream;
    use hyper::server::Request as HyperRequest;
    use hyper::header::{Headers, ContentDisposition, DispositionParam, ContentType,
                        DispositionType};
    use mime::{Mime, TopLevel, SubLevel};

    use mock::MockStream;

    #[test]
    fn parser() {
        let input = b"POST / HTTP/1.1\r\n\
                      Host: example.domain\r\n\
                      Content-Type: multipart/form-data; boundary=\"abcdefg\"\r\n\
                      Content-Length: 1000\r\n\
                      \r\n\
                      --abcdefg\r\n\
                      Content-Disposition: form-data; name=\"field1\"\r\n\
                      \r\n\
                      data1\r\n\
                      --abcdefg\r\n\
                      Content-Disposition: form-data; name=\"field2\"; filename=\"image.gif\"\r\n\
                      Content-Type: image/gif\r\n\
                      \r\n\
                      This is a file\r\n\
                      with two lines\r\n\
                      --abcdefg\r\n\
                      Content-Disposition: form-data; name=\"field3\"; filename=\"file.txt\"\r\n\
                      \r\n\
                      This is a file\r\n\
                      --abcdefg--";

        let mut mock = MockStream::with_input(input);

        let mock: &mut dyn NetworkStream = &mut mock;
        let mut stream = BufReader::new(mock);
        let sock: SocketAddr = "127.0.0.1:80".parse().unwrap();
        let req = HyperRequest::new(&mut stream, sock).unwrap();
        let (_, _, headers, _, _, mut reader) = req.deconstruct();

        match read_formdata(&mut reader, &headers) {
            Ok(form_data) => {
                assert_eq!(form_data.fields.len(), 1);
                for (key, val) in form_data.fields {
                    if &key == "field1" {
                        assert_eq!(&val, "data1");
                    }
                }

                assert_eq!(form_data.files.len(), 2);
                for (key, file) in form_data.files {
                    if &key == "field2" {
                        assert_eq!(file.size, Some(30));
                        assert_eq!(&*file.filename().unwrap().unwrap(), "image.gif");
                        assert_eq!(file.content_type().unwrap(), mime!(Image/Gif));
                    } else if &key == "field3" {
                        assert_eq!(file.size, Some(14));
                        assert_eq!(&*file.filename().unwrap().unwrap(), "file.txt");
                        assert!(file.content_type().is_none());
                    }
                }
            },
            Err(err) => panic!("{}", err),
        }
    }

    #[test]
    fn multi_file_parser() {
        let input = b"POST / HTTP/1.1\r\n\
                      Host: example.domain\r\n\
                      Content-Type: multipart/form-data; boundary=\"abcdefg\"\r\n\
                      Content-Length: 1000\r\n\
                      \r\n\
                      --abcdefg\r\n\
                      Content-Disposition: form-data; name=\"field1\"\r\n\
                      \r\n\
                      data1\r\n\
                      --abcdefg\r\n\
                      Content-Disposition: form-data; name=\"field2\"; filename=\"image.gif\"\r\n\
                      Content-Type: image/gif\r\n\
                      \r\n\
                      This is a file\r\n\
                      with two lines\r\n\
                      --abcdefg\r\n\
                      Content-Disposition: form-data; name=\"field2\"; filename=\"file.txt\"\r\n\
                      \r\n\
                      This is a file\r\n\
                      --abcdefg--";

        let mut mock = MockStream::with_input(input);

        let mock: &mut dyn NetworkStream = &mut mock;
        let mut stream = BufReader::new(mock);
        let sock: SocketAddr = "127.0.0.1:80".parse().unwrap();
        let req = HyperRequest::new(&mut stream, sock).unwrap();
        let (_, _, headers, _, _, mut reader) = req.deconstruct();

        match read_formdata(&mut reader, &headers) {
            Ok(form_data) => {
                assert_eq!(form_data.fields.len(), 1);
                for (key, val) in form_data.fields {
                    if &key == "field1" {
                        assert_eq!(&val, "data1");
                    }
                }

                assert_eq!(form_data.files.len(), 2);
                let (ref key, ref file) = form_data.files[0];

                assert_eq!(key, "field2");
                assert_eq!(file.size, Some(30));
                assert_eq!(&*file.filename().unwrap().unwrap(), "image.gif");
                assert_eq!(file.content_type().unwrap(), mime!(Image/Gif));

                let (ref key, ref file) = form_data.files[1];
                assert!(key == "field2");
                assert_eq!(file.size, Some(14));
                assert_eq!(&*file.filename().unwrap().unwrap(), "file.txt");
                assert!(file.content_type().is_none());

            },
            Err(err) => panic!("{}", err),
        }
    }

    #[test]
    fn mixed_parser() {
        let input = b"POST / HTTP/1.1\r\n\
                      Host: example.domain\r\n\
                      Content-Type: multipart/form-data; boundary=AaB03x\r\n\
                      Content-Length: 1000\r\n\
                      \r\n\
                      --AaB03x\r\n\
                      Content-Disposition: form-data; name=\"submit-name\"\r\n\
                      \r\n\
                      Larry\r\n\
                      --AaB03x\r\n\
                      Content-Disposition: form-data; name=\"files\"\r\n\
                      Content-Type: multipart/mixed; boundary=BbC04y\r\n\
                      \r\n\
                      --BbC04y\r\n\
                      Content-Disposition: file; filename=\"file1.txt\"\r\n\
                      \r\n\
                      ... contents of file1.txt ...\r\n\
                      --BbC04y\r\n\
                      Content-Disposition: file; filename=\"awesome_image.gif\"\r\n\
                      Content-Type: image/gif\r\n\
                      Content-Transfer-Encoding: binary\r\n\
                      \r\n\
                      ... contents of awesome_image.gif ...\r\n\
                      --BbC04y--\r\n\
                      --AaB03x--";

        let mut mock = MockStream::with_input(input);

        let mock: &mut dyn NetworkStream = &mut mock;
        let mut stream = BufReader::new(mock);
        let sock: SocketAddr = "127.0.0.1:80".parse().unwrap();
        let req = HyperRequest::new(&mut stream, sock).unwrap();
        let (_, _, headers, _, _, mut reader) = req.deconstruct();

        match read_formdata(&mut reader, &headers) {
            Ok(form_data) => {
                assert_eq!(form_data.fields.len(), 1);
                for (key, val) in form_data.fields {
                    if &key == "submit-name" {
                        assert_eq!(&val, "Larry");
                    }
                }

                assert_eq!(form_data.files.len(), 2);
                for (key, file) in form_data.files {
                    assert_eq!(&key, "files");
                    match &file.filename().unwrap().unwrap()[..] {
                        "file1.txt" => {
                            assert_eq!(file.size, Some(29));
                            assert!(file.content_type().is_none());
                        }
                        "awesome_image.gif" => {
                            assert_eq!(file.size, Some(37));
                            assert_eq!(file.content_type().unwrap(), mime!(Image/Gif));
                        },
                        _ => unreachable!(),
                    }
                }
            },
            Err(err) => panic!("{}", err),
        }
    }

    #[test]
    fn simple_writer() {
        // Create a simple short file for testing
        let tmpdir = tempdir::TempDir::new("formdata_test").unwrap();
        let tmppath = tmpdir.path().join("testfile");
        let mut tmpfile = File::create(tmppath.clone()).unwrap();
        writeln!(tmpfile, "this is example file content").unwrap();

        let mut photo_headers = Headers::new();
        photo_headers.set(ContentType(Mime(TopLevel::Image, SubLevel::Gif, vec![])));
        photo_headers.set(ContentDisposition {
            disposition: DispositionType::Ext("form-data".to_owned()),
            parameters: vec![DispositionParam::Ext("name".to_owned(), "photo".to_owned()),
                             DispositionParam::Ext("filename".to_owned(), "mike.gif".to_owned())],
        });

        let formdata = FormData {
            fields: vec![ ("name".to_owned(), "Mike".to_owned()),
                            ("age".to_owned(), "46".to_owned()) ],
            files: vec![ ("photo".to_owned(), FilePart::new(photo_headers, &tmppath)) ],
        };

        let mut output: Vec<u8> = Vec::new();
        let boundary = generate_boundary();
        match write_formdata(&mut output, &boundary, &formdata) {
            Ok(count) => assert_eq!(count, 568),
            Err(e) => panic!("Unable to write formdata: {}", e),
        }

        println!("{}", String::from_utf8_lossy(&output));
    }


    #[test]
    fn chunked_writer() {
        // Create a simple short file for testing
        let tmpdir = tempdir::TempDir::new("formdata_test").unwrap();
        let tmppath = tmpdir.path().join("testfile");
        let mut tmpfile = File::create(tmppath.clone()).unwrap();
        writeln!(tmpfile, "this is example file content").unwrap();

        let mut photo_headers = Headers::new();
        photo_headers.set(ContentType(Mime(TopLevel::Image, SubLevel::Gif, vec![])));
        photo_headers.set(ContentDisposition {
            disposition: DispositionType::Ext("form-data".to_owned()),
            parameters: vec![DispositionParam::Ext("name".to_owned(), "photo".to_owned()),
                             DispositionParam::Ext("filename".to_owned(), "mike.gif".to_owned())],
        });

        let formdata = FormData {
            fields: vec![ ("name".to_owned(), "Mike".to_owned()),
                            ("age".to_owned(), "46".to_owned()) ],
            files: vec![ ("photo".to_owned(), FilePart::new(photo_headers, &tmppath)) ],
        };

        let mut output: Vec<u8> = Vec::new();
        let boundary = generate_boundary();
        assert!(write_formdata_chunked(&mut output, &boundary, &formdata).is_ok());
        println!("{}", String::from_utf8_lossy(&output));
    }
}
