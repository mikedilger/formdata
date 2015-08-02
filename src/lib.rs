//! This crate parses and processes `hyper::server::Request` data that contains
//! `multipart/form-data` content.
//!
//! The main entry point is `parse_multipart`

extern crate httparse;
extern crate hyper;
extern crate iron;
extern crate libc;
#[macro_use]
extern crate mime;
extern crate tempdir;
extern crate textnonce;
#[macro_use]
extern crate log;

pub mod buf;
pub mod error;
mod headers;
#[cfg(test)]
mod mock;

use std::path::PathBuf;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};

use hyper::header::{ContentType, Headers};
use hyper::server::Request as HyperRequest;
use mime::{Attr, Mime, Param, SubLevel, TopLevel, Value};
use tempdir::TempDir;
use textnonce::TextNonce;
use iron::Request as IronRequest;

use buf::BufReadExt;
pub use error::Error;
use headers::ContentDisposition;

/// An uploaded file that was received as part of `multipart/form-data` parsing.
///
/// Files are streamed to disk because they may not fit in memory.
#[derive(Clone, Debug, PartialEq)]
pub struct UploadedFile {
    /// The temporary file where the data was saved.
    pub path: PathBuf,
    /// The filename that was specified in the data, unfiltered. It may or may not be legal on the
    /// local filesystem.
    pub filename: Option<String>,
    /// The unvalidated content-type that was specified in the data.
    pub content_type: Mime,
    /// The size of the file.
    pub size: usize,
}

/// The extracted text fields and uploaded files from a `multipart/form-data` request.
///
/// Use `parse_multipart` to devise this object from a request.
#[derive(Clone, Debug, PartialEq)]
pub struct FormData {
    /// Key-value pairs for plain text fields. Technically, these are form data parts with no
    /// filename specified in the part's `Content-Disposition`.
    pub fields: Vec<(String, String)>,
    /// Key-value pairs for temporary files. Technically, these are form data parts with a filename
    /// specified in the part's `Content-Disposition`.
    pub files: Vec<(String, UploadedFile)>,
}

impl FormData {
    pub fn new() -> FormData {
        FormData { fields: vec![], files: vec![] }
    }
}

/// Parses and processes `hyper::server::Request` data that is `multipart/form-data` content.
///
/// The request is streamed, and this function saves embedded uploaded files to disk as they are
/// encountered by the parser.
pub fn parse_multipart(request: &mut Request) -> Result<FormData, Error> {
    let boundary = try!(get_boundary(request));
    trace!("Boundary is {}", boundary);
    let mut reader = BufReader::with_capacity(4096, request.read_mut());
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

                        let cd: &ContentDisposition = match headers.get() {
                            Some(cd) => cd,
                            None => return Err(Error::MissingDisposition),
                        };

                        let ct: Option<&ContentType> = headers.get();

                        match mode {
                            FormData if cd.disposition == "form-data" => {
                                state = if is_multipart_mixed(ct) {
                                    CapturingMixed(cd.clone(), ct.unwrap().clone())
                                } else if ct.is_some() || cd.filename.is_some() {
                                    CapturingFile(cd.clone(), ct.map(|ct| ct.clone()))
                                } else {
                                    CapturingValue(cd.clone())
                                };
                            },
                            Mixed(_) if cd.disposition == "file" => {
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
                let boundary = try!(get_boundary_token(&(ct.0).2));
                let mode = Mixed(try!(cd.name.ok_or(Error::NoName)));
                try!(run_state_machine(boundary, reader, form_data, mode));
                state = Discarding;
            },
            CapturingValue(_) if mode != FormData => unreachable!(),
            CapturingValue(cd) => {
                buf.truncate(0);
                let _ = try!(reader.stream_until_token(&crlf_boundary, &mut buf));

                let key = try!(cd.name.ok_or(Error::NoName));
                let val = try!(String::from_utf8(buf));

                form_data.fields.push((key, val));

                state = ReadingHeaders;
            },
            CapturingFile(cd, ct) => {
                // Setup a file to capture the contents.
                let mut path = try!(TempDir::new("formdata")).into_path();
                path.push(TextNonce::sized_urlsafe(32).unwrap().into_string());
                let mut file = try!(File::create(path.clone()));

                // Stream out the file.
                let read = try!(reader.stream_until_token(&crlf_boundary, &mut file));

                // TODO: Handle Content-Transfer-Encoding.

                let key = match mode {
                    Mixed(ref name) => name.clone(),
                    FormData => try!(cd.name.ok_or(Error::NoName)),
                };

                let file = UploadedFile {
                    path: path,
                    filename: cd.filename.clone(),
                    content_type: ct.map_or(mime!(Text/Plain; Charset=Utf8), |ct| ct.0),
                    size: read - crlf_boundary.len()
                };

                form_data.files.push((key, file));
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

fn get_boundary(request: &Request) -> Result<String, Error> {
    // Verify that the request is 'Content-Type: multipart/form-data'.
    let ct: &ContentType = match request.headers().get() {
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

fn get_boundary_token(params: &[Param]) -> Result<String, Error> {
    for &(ref attr, ref val) in params.iter() {
        if let (&Attr::Boundary, &Value::Ext(ref val)) = (attr, val) {
            return Ok(format!("--{}", val.clone()));
        }
    }

    Err(Error::BoundaryNotSpecified)
}

fn crlf_boundary(boundary: &Vec<u8>) -> Vec<u8> {
    let mut crlf_boundary = Vec::with_capacity(2 + boundary.len());
    crlf_boundary.extend(b"\r\n".iter().map(|&i| i));
    crlf_boundary.extend(boundary.clone());
    crlf_boundary
}

/// A wrapper for request data to provide parsing multipart requests to any front-end that provides
/// a `hyper::header::Headers` and a `std::io::Read` of the request's entire body.
pub trait Request {
    /// Returns a reference to the request's headers.
    fn headers(&self) -> &Headers;

    /// Returns a mutable reference to the request's body reader.
    fn read_mut(&mut self) -> &mut Read;
}

impl<'a,'b> Request for HyperRequest<'a,'b> {
    fn headers(&self) -> &Headers {
        &self.headers
    }

    fn read_mut(&mut self) -> &mut Read {
        self
    }
}

impl<'a,'b> Request for IronRequest<'a,'b> {
    fn headers(&self) -> &Headers {
        &self.headers
    }

    fn read_mut(&mut self) -> &mut Read {
        &mut self.body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std;
    use std::net::SocketAddr;
    use iron;

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
        let mut req = HyperRequest::new(&mut stream, sock).unwrap();

        match parse_multipart(&mut req) {
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
                        assert_eq!(&file.filename.unwrap(), "image.gif");
                        assert_eq!(file.content_type, mime!(Image/Gif));
                    } else if &key == "field3" {
                        assert_eq!(file.size, 14);
                        assert_eq!(&file.filename.unwrap(), "file.txt");
                        assert_eq!(file.content_type, mime!(Text/Plain; Charset=Utf8));
                    }
                }
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

        match parse_multipart(&mut req) {
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
                    match &file.filename.unwrap()[..] {
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
    fn parser_iron() {
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
        let mut _req = HyperRequest::new(&mut stream, sock).unwrap();
        let mut req = iron::Request::from_http(
            _req,
            SocketAddr::V4(
                std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(127, 0, 0, 1), 11218),
            ),
            &iron::Protocol::Http
        ).unwrap();
        match parse_multipart(&mut req) {
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
                        assert_eq!(&file.filename.unwrap(), "image.gif");
                        assert_eq!(file.content_type, mime!(Image/Gif));
                    } else if &key == "field3" {
                        assert_eq!(file.size, 14);
                        assert_eq!(&file.filename.unwrap(), "file.txt");
                        assert_eq!(file.content_type, mime!(Text/Plain; Charset=Utf8));
                    }
                }
            },
            Err(err) => panic!("{}", err),
        }
    }

    #[test]
    fn mixed_parser_iron() {
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
        let mut _req = HyperRequest::new(&mut stream, sock).unwrap();
        let mut req = iron::Request::from_http(
            _req,
            SocketAddr::V4(
                std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(127, 0, 0, 1), 12321),
            ),
            &iron::Protocol::Http
        ).unwrap();

        match parse_multipart(&mut req) {
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
                    match &file.filename.unwrap()[..] {
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
}
