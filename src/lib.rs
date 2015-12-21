// Copyright © 2015 by Michael Dilger (of New Zealand)
// This code is licensed under the MIT license (see LICENSE-MIT for details)

//! This crate parses and processes a stream of data that contains
//! `multipart/form-data` content.
//!
//! The main entry point is `parse_multipart`

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

pub mod buf;
pub mod error;
#[cfg(test)]
mod mock;
pub mod uploaded_file;
pub mod form_data;

pub use error::Error;
pub use uploaded_file::UploadedFile;
pub use form_data::FormData;

use encoding::all;
use encoding::{Encoding, DecoderTrap};
use std::borrow::Cow;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use hyper::header::{ContentType, Headers, ContentDisposition, DispositionType,
                    DispositionParam, Charset};
use mime::{Attr, Mime, Param, SubLevel, TopLevel, Value};
use buf::BufReadExt;

/// Parses and processes a stream of `multipart/form-data` content.
///
/// The request is streamed, and this function saves embedded uploaded files to disk as they are
/// encountered by the parser.
pub fn parse_multipart<S: Read>(stream: &mut S, boundary: String) -> Result<FormData, Error> {
    let mut reader = BufReader::with_capacity(4096, stream);
    let mut form_data = FormData::new();
    try!(run_state_machine(boundary, &mut reader, &mut form_data, MultipartSubLevel::FormData));
    Ok(form_data)
}

// A state in the parsing state machine.
enum State {
    // Discard until after initial boundary and CRLF.
    Discarding,
    // Capture headers to blank line.
    ReadingHeaders,
    // Capture entire `multipart/mixed` body until boundary, then discard past CRLF.
    // "Content Disposition Header for Each Part" https://tools.ietf.org/html/rfc7578#section-4.2
    CapturingMixed(ContentDisposition, ContentType),
    // Capture value until boundary, then discard past CRLF.
    CapturingValue(ContentDisposition),
    // Capture file until boundary, then discard past CRLF.
    CapturingFile(ContentDisposition, Option<ContentType>),
}

// A multipart MIME parsing mode.
#[derive(PartialEq)]
enum MultipartSubLevel {
    // Represents `multipart/form-data`.
    FormData,
    // Represents `multipart/mixed` with a `name` key.
    Mixed(String),
}

// Parse either a `multipart/form-data` or `multipart/mixed` MIME body.
fn run_state_machine<R: BufRead>(boundary: String, reader: &mut R, form_data: &mut FormData,
                                 mode: MultipartSubLevel) -> Result<(), Error> {
    use State::*;
    use MultipartSubLevel::*;

    let boundary = boundary.into_bytes();
    let crlf_boundary = crlf_boundary(&boundary);
    let mut state = Discarding;

    loop {
        let mut buf: Vec<u8> = Vec::new();

        match state {
            Discarding => {
                // Read up to and including the boundary.
                let read = try!(reader.stream_until_token(&boundary, &mut buf));
                if read == 0 {
                    return Err(Error::Eof);
                }

                state = ReadingHeaders;
            },
            ReadingHeaders => {
                {
                    // If the next two lookahead characters are '--', parsing is finished.
                    let peeker = try!(reader.fill_buf());
                    if peeker.len() >= 2 && &peeker[..2] == b"--" {
                        return Ok(());
                    }
                }

                // Read up to and including the CRLF after the boundary.
                let read = try!(reader.stream_until_token(b"\r\n", &mut buf));
                if read == 0 {
                    return Err(Error::Eof);
                }

                buf.truncate(0);
                let read = try!(reader.stream_until_token(b"\r\n\r\n", &mut buf));
                if read == 0 {
                    return Err(Error::Eof);
                }
                // `parse_headers` requires this token at the end:
                buf.extend(b"\r\n\r\n".iter().map(|&i| i));

                // Parse the headers.
                let mut header_memory = [httparse::EMPTY_HEADER; 4];
                match httparse::parse_headers(&buf, &mut header_memory) {
                    Ok(httparse::Status::Complete((_, raw_headers))) => {
                        // Turn raw headers into hyper headers.
                        let headers = try!(Headers::from_raw(raw_headers));

                        // Each part must contain a Content Disposition Header field
                        // https://tools.ietf.org/html/rfc7578#section-4.2
                        let cd: &ContentDisposition = match headers.get() {
                            Some(cd) => cd,
                            None => return Err(Error::MissingDisposition),
                        };
                        let cd_filename: Option<String> =
                            try!(get_content_disposition_filename(cd));

                        let ct: Option<&ContentType> = headers.get();

                        match mode {
                            FormData if cd.disposition ==
                                DispositionType::Ext("form-data".to_owned()) => {
                                state = if is_multipart_mixed(ct) {
                                    CapturingMixed(cd.clone(), ct.unwrap().clone())
                                } else if ct.is_some() || cd_filename.is_some() {
                                    CapturingFile(cd.clone(), ct.map(|ct| ct.clone()))
                                } else {
                                    CapturingValue(cd.clone())
                                };
                            },
                            Mixed(_) if cd.disposition == DispositionType::Ext("file".to_owned())
                                || cd.disposition == DispositionType::Attachment => {
                                state = CapturingFile(cd.clone(), ct.map(|ct| ct.clone()));
                            },
                            _ => return Err(Error::InvalidDisposition),
                        }
                    },
                    Ok(httparse::Status::Partial) => return Err(Error::PartialHeaders),
                    Err(err) => return Err(From::from(err)),
                }
            },
            CapturingMixed(cd, ct) => {
                let cd_name: Option<String> = get_content_disposition_name(&cd);
                let boundary = try!(get_boundary_token(&(ct.0).2));
                let mode = Mixed(try!(cd_name.ok_or(Error::NoName)));
                try!(run_state_machine(boundary, reader, form_data, mode));
                state = Discarding;
            },
            CapturingValue(_) if mode != FormData => unreachable!(),
            CapturingValue(cd) => {
                let cd_name: Option<String> = get_content_disposition_name(&cd);
                buf.truncate(0);
                let _ = try!(reader.stream_until_token(&crlf_boundary, &mut buf));

                let key = try!(cd_name.ok_or(Error::NoName));
                let val = try!(String::from_utf8(buf));

                form_data.fields.push((key, val));

                state = ReadingHeaders;
            },
            CapturingFile(cd, ct) => {
                let cd_name: Option<String> = get_content_disposition_name(&cd);
                let cd_filename: Option<String> = try!(get_content_disposition_filename(&cd));

                // Setup a file to capture the contents.
                let mut uploaded_file = try!(UploadedFile::new(
                    ct.map_or(mime!(Text/Plain; Charset=Utf8), |ct| ct.0)));
                uploaded_file.filename = cd_filename;
                let mut file = try!(File::create(uploaded_file.path.clone()));

                // Stream out the file.
                let read = try!(reader.stream_until_token(&crlf_boundary, &mut file));
                uploaded_file.size = read - crlf_boundary.len();

                // TODO: Handle Content-Transfer-Encoding.  RFC 7578 section 4.7 deprecated
                // this, and the authors state "Currently, no deployed implementations that
                // send such bodies have been discovered", so this is very low priority.

                let key = match mode {
                    Mixed(ref name) => name.clone(),
                    FormData => try!(cd_name.ok_or(Error::NoName)),
                };

                form_data.files.push((key, uploaded_file));
                state = ReadingHeaders;
            },
        }
    }
}

fn is_multipart_mixed(ct: Option<&ContentType>) -> bool {
    let ct = match ct {
        Some(ct) => ct,
        None => return false,
    };

    let ContentType(ref mime) = *ct;
    let Mime(ref top_level, ref sub_level, _) = *mime;

    if *top_level != TopLevel::Multipart {
        return false;
    }

    match *sub_level {
        SubLevel::Ext(ref ext) => ext == "mixed",
        _ => false,
    }
}

/// Get the `multipart/form-data` boundary string from hyper::Headers
pub fn get_multipart_boundary(headers: &Headers) -> Result<String, Error> {
    // Verify that the request is 'Content-Type: multipart/form-data'.
    let ct: &ContentType = match headers.get() {
        Some(ct) => ct,
        None => return Err(Error::NoRequestContentType),
    };
    let ContentType(ref mime) = *ct;
    let Mime(ref top_level, ref sub_level, ref params) = *mime;

    if *top_level != TopLevel::Multipart {
        return Err(Error::NotMultipart);
    }

    if *sub_level != SubLevel::FormData {
        return Err(Error::NotFormData);
    }

    get_boundary_token(params)
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

#[inline]
fn get_content_disposition_filename(cd: &ContentDisposition) -> Result<Option<String>, Error> {
    if let Some(&DispositionParam::Filename(ref charset, _, ref bytes)) =
        cd.parameters.iter().find(|&x| match *x {
            DispositionParam::Filename(_,_,_) => true,
            _ => false,
        })
    {
        match charset_decode(charset, bytes) {
            Ok(filename) => Ok(Some(filename)),
            Err(e) => Err(Error::Decoding(e)),
        }
    } else {
        Ok(None)
    }
}

fn get_boundary_token(params: &[Param]) -> Result<String, Error> {
    for &(ref attr, ref val) in params.iter() {
        if let (&Attr::Boundary, &Value::Ext(ref val)) = (attr, val) {
            return Ok(format!("--{}", val.clone()));
        }
    }

    Err(Error::BoundaryNotSpecified)
}

// https://tools.ietf.org/html/rfc7578#section-4.1 (`--` is included in the boundary param)
fn crlf_boundary(boundary: &Vec<u8>) -> Vec<u8> {
    let mut crlf_boundary = Vec::with_capacity(2 + boundary.len());
    crlf_boundary.extend(b"\r\n".iter().map(|&i| i));
    crlf_boundary.extend(boundary.clone());
    crlf_boundary
}

// This decodes bytes encoded according to a hyper::header::Charset encoding, using the
// rust-encoding crate.  Only supports encodings defined in both crates.
fn charset_decode(charset: &Charset, bytes: &Vec<u8>) -> Result<String, Cow<'static, str>> {
    Ok(match *charset {
        Charset::Us_Ascii => try!(all::ASCII.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_8859_1 => try!(all::ISO_8859_1.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_8859_2 => try!(all::ISO_8859_2.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_8859_3 => try!(all::ISO_8859_3.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_8859_4 => try!(all::ISO_8859_4.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_8859_5 => try!(all::ISO_8859_5.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_8859_6 => try!(all::ISO_8859_6.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_8859_7 => try!(all::ISO_8859_7.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_8859_8 => try!(all::ISO_8859_8.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_8859_9 => return Err("ISO_8859_9 is not supported".into()),
        Charset::Iso_8859_10 => try!(all::ISO_8859_10.decode(bytes, DecoderTrap::Strict)),
        Charset::Shift_Jis => return Err("Shift_Jis is not supported".into()),
        Charset::Euc_Jp => try!(all::EUC_JP.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_2022_Kr => return Err("Iso_2022_Kr is not supported".into()),
        Charset::Euc_Kr => return Err("Euc_Kr is not supported".into()),
        Charset::Iso_2022_Jp => try!(all::ISO_2022_JP.decode(bytes, DecoderTrap::Strict)),
        Charset::Iso_2022_Jp_2 => return Err("Iso_2022_Jp_2 is not supported".into()),
        Charset::Iso_8859_6_E => return Err("Iso_8859_6_E is not supported".into()),
        Charset::Iso_8859_6_I => return Err("Iso_8859_6_I is not supported".into()),
        Charset::Iso_8859_8_E => return Err("Iso_8859_8_E is not supported".into()),
        Charset::Iso_8859_8_I => return Err("Iso_8859_8_I is not supported".into()),
        Charset::Gb2312 => return Err("Gb2312 is not supported".into()),
        Charset::Big5 => try!(all::BIG5_2003.decode(bytes, DecoderTrap::Strict)),
        Charset::Koi8_R => try!(all::KOI8_R.decode(bytes, DecoderTrap::Strict)),
        Charset::Ext(ref s) => match &**s {
            "UTF-8" => try!(all::UTF_8.decode(bytes, DecoderTrap::Strict)),
            _ => return Err("Encoding is not supported".into()),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::net::SocketAddr;

    use hyper::buffer::BufReader;
    use hyper::net::NetworkStream;
    use hyper::server::Request as HyperRequest;

    use mock::MockStream;

    use serde_json;

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
        let mut req = HyperRequest::new(&mut stream, sock).unwrap();
        let boundary = get_multipart_boundary(&req.headers).unwrap();

        match parse_multipart(&mut req, boundary) {
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
                        assert_eq!(file.size, 30);
                        assert_eq!(file.filename.as_ref().unwrap(), "image.gif");
                        assert_eq!(file.content_type, mime!(Image/Gif));
                    } else if &key == "field3" {
                        assert_eq!(file.size, 14);
                        assert_eq!(file.filename.as_ref().unwrap(), "file.txt");
                        assert_eq!(file.content_type, mime!(Text/Plain; Charset=Utf8));
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
        let mut req = HyperRequest::new(&mut stream, sock).unwrap();
        let boundary = get_multipart_boundary(&req.headers).unwrap();

        match parse_multipart(&mut req, boundary) {
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
                assert_eq!(file.size, 30);
                assert_eq!(file.filename.as_ref().unwrap(), "image.gif");
                assert_eq!(file.content_type, mime!(Image/Gif));

                let (ref key, ref file) = form_data.files[1];
                assert!(key == "field2");
                assert_eq!(file.size, 14);
                assert_eq!(file.filename.as_ref().unwrap(), "file.txt");
                assert_eq!(file.content_type, mime!(Text/Plain; Charset=Utf8));

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
        let mut req = HyperRequest::new(&mut stream, sock).unwrap();
        let boundary = get_multipart_boundary(&req.headers).unwrap();

        match parse_multipart(&mut req, boundary) {
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
                    match &file.filename.as_ref().unwrap()[..] {
                        "file1.txt" => {
                            assert_eq!(file.size, 29);
                            assert_eq!(file.content_type, mime!(Text/Plain; Charset=Utf8));
                        }
                        "awesome_image.gif" => {
                            assert_eq!(file.size, 37);
                            assert_eq!(file.content_type, mime!(Image/Gif));
                        },
                        _ => unreachable!(),
                    }
                }
            },
            Err(err) => panic!("{}", err),
        }
    }

    #[test]
    fn test_serde_uploaded_file() {
        let uploaded_file = UploadedFile::new(
            "text/html; charset=utf-8".parse().unwrap() ).unwrap();
        let serialized = serde_json::to_string(&uploaded_file).unwrap();
        let deserialized: UploadedFile = serde_json::from_str(&serialized).unwrap();
        assert_eq!(uploaded_file, deserialized);
    }

    #[test]
    fn test_serde_form_data() {
        let form_data = FormData {
            fields: vec![
                ("name".to_owned(), "Betty".to_owned()),
                ("age".to_owned(), "32".to_owned()) ],
            files: vec![
                ("test.txt".to_owned(), UploadedFile::new(
                    "text/html; charset=utf-8".parse().unwrap()).unwrap()),
                ("test.txt".to_owned(), UploadedFile::new(
                    "text/html; charset=utf-8".parse().unwrap()).unwrap()),
                ],
        };
        let serialized = serde_json::to_string(&form_data).unwrap();
        let deserialized: FormData = serde_json::from_str(&serialized).unwrap();
        assert_eq!(form_data, deserialized);
    }
}
