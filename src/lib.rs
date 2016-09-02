// Copyright © 2015 by Michael Dilger (of New Zealand)
// This code is licensed under the MIT license (see LICENSE-MIT for details)

//! This crate parses and processes a stream of data that contains
//! `multipart/form-data` content.
//!
//! The main entry point is `read_formdata`

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
extern crate encoding;

extern crate mime_multipart;

mod error;
mod form_data;
#[cfg(test)]
mod mock;

pub use error::Error;
pub use form_data::FormData;

use std::io::Read;
use hyper::header::{Headers, ContentDisposition, DispositionParam};
use mime_multipart::Node;

/// Parse MIME `multipart/form-data` information from a stream as a `FormData`.
pub fn read_formdata<S: Read>(stream: &mut S, headers: &Headers) -> Result<FormData, Error>
{
    let nodes = try!(mime_multipart::read_multipart_body(stream, headers, false));

    let mut formdata = FormData::new();
    try!(fill_formdata(&mut formdata, nodes));
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
                let key = try!(cd_name.ok_or(Error::NoName));
                let val = try!(String::from_utf8(part.body));
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
                let key = try!(cd_name.ok_or(Error::NoName));
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
                let key = try!(cd_name.ok_or(Error::NoName));
                for node in nodes {
                    match node {
                        Node::Part(part) => {
                            let val = try!(String::from_utf8(part.body));
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


#[cfg(test)]
mod tests {
    use super::*;

    use std::net::SocketAddr;

    use hyper::buffer::BufReader;
    use hyper::net::NetworkStream;
    use hyper::server::Request as HyperRequest;

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

        let mock: &mut NetworkStream = &mut mock;
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

        let mock: &mut NetworkStream = &mut mock;
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

        let mock: &mut NetworkStream = &mut mock;
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
}
