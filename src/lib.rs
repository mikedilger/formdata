//! This crate parses and processes `hyper::server::Request` data that contains
//! `multipart/form-data` content.
//!
//! The main entry point is `parse_multipart`

extern crate httparse;
extern crate hyper;
extern crate libc;
#[macro_use]
extern crate mime;
extern crate tempdir;
extern crate textnonce;

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
use mime::{Attr, Mime, SubLevel, TopLevel, Value};
use tempdir::TempDir;
use textnonce::TextNonce;

use buf::BufReadPlus;
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

/// Parses and processes `hyper::server::Request` data is `multipart/form-data` content.
///
/// The request is streamed, and this function saves embedded uploaded files to disk as they are
/// encountered by the parser.
///
/// This function returns two sets of data. The first, `Vec<(String, String)>`, are the
/// variable-value pairs from the POST-ed form. The second `Vec<(String, UploadedFile)>` are the
/// variable-file pairs from the POST-ed form, where `UploadedFile` structures describe the
/// uploaded file.
pub fn parse_multipart(request: &mut Request)
                       -> Result<(Vec<(String, String)>, Vec<(String, UploadedFile)>), Error> {
    let mut parameters: Vec<(String, String)> = Vec::new();
    let mut files: Vec<(String, UploadedFile)> = Vec::new();

    let string_boundary = try!(get_boundary(request));
    let boundary: Vec<u8> = string_boundary.into_bytes();
    let mut crlf_boundary: Vec<u8> = Vec::with_capacity(2 + boundary.len());
    crlf_boundary.extend(b"\r\n".iter().map(|&i| i));
    crlf_boundary.extend(boundary.clone());

    let mut reader = BufReader::with_capacity(4096, request);

    enum State {
        // Discard until after initial boundary and CRLF.
        Discarding,
        // Capture headers to blank line.
        ReadingHeaders,
        // Capture value until boundary, then discard past CRLF.
        CapturingValue,
        // Capture file until boundary, then discard past CRLF.
        CapturingFile,
    }

    let mut state = State::Discarding;
    let mut headers = Headers::new();

    loop {
        let mut buf: Vec<u8> = Vec::new();

        match state {
            State::Discarding => {
                // Read up to and including the boundary
                let read = try!(reader.stream_until_token(&*boundary, &mut buf));
                if read == 0 {
                    return Err(Error::Eof);
                }

                state = State::ReadingHeaders;
            },
            State::ReadingHeaders => {
                {
                    // If the next two lookahead characters are '--', parsing is finished.
                    let peeker = try!(reader.fill_buf());
                    if peeker.len() >= 2 && &peeker[..2] == b"--" {
                        return Ok((parameters, files));
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
                match httparse::parse_headers(&*buf, &mut header_memory) {
                    Ok(httparse::Status::Complete((_, raw_headers))) => {
                        // Turn raw headers into hyper headers.
                        headers = try!(Headers::from_raw(raw_headers));

                        let cd: &ContentDisposition = match headers.get() {
                            Some(cd) => cd,
                            None => return Err(Error::MissingDisposition),
                        };
                        let ct: Option<&ContentType> = headers.get();

                        state = if ct.is_some() || cd.filename.is_some() {
                            State::CapturingFile
                        } else {
                            State::CapturingValue
                        };
                    },
                    Ok(httparse::Status::Partial) => {
                        return Err(Error::PartialHeaders);
                    },
                    Err(err) => return Err(From::from(err)),
                }
            },
            State::CapturingValue => {
                buf.truncate(0);
                let _ = try!(reader.stream_until_token(&*crlf_boundary, &mut buf));

                let cd: &ContentDisposition = headers.get().unwrap();

                let key = match cd.name {
                    None => return Err(Error::NoName),
                    Some(ref name) => name.clone(),
                };
                let val = try!(String::from_utf8(buf));

                parameters.push((key, val));

                state = State::ReadingHeaders;
            },
            State::CapturingFile => {
                // Setup a file to capture the contents.
                let mut path = try!(TempDir::new("formdata")).into_path();
                path.push(TextNonce::sized_urlsafe(32).unwrap().into_string());
                let mut file = try!(File::create(path.clone()));

                // Stream out the file.
                let read = try!(reader.stream_until_token(&*crlf_boundary, &mut file));

                let cd: &ContentDisposition = headers.get().unwrap();
                let key = match cd.name {
                    None => return Err(Error::NoName),
                    Some(ref name) => name.clone()
                };

                let ct = match headers.get::<ContentType>() {
                    Some(ct) => (**ct).clone(),
                    None => mime!(Text/Plain; Charset=Utf8)
                };

                // TODO: Handle content-type: multipart/mixed as multiple files.
                // TODO: Handle content-transfer-encoding.

                let file = UploadedFile {
                    path: path,
                    filename: cd.filename.clone(),
                    content_type: ct,
                    size: read - crlf_boundary.len()
                };
                files.push((key, file));

                state = State::ReadingHeaders;
            },
        }
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

    // Get the boundary token.
    for &(ref attr, ref val) in params.iter() {
        if let (&Attr::Boundary, &Value::Ext(ref val)) = (attr, val) {
            return Ok(format!("--{}", val.clone()));
        }
    }

    Err(Error::BoundaryNotSpecified)

}

/// A wrapper for request data to provide parsing multipart requests to any front-end that provides
/// a `hyper::header::Headers` and a `std::io::Read` of the request's entire body.
pub trait Request: Read {
    /// Returns a reference to the request's headers.
    fn headers(&self) -> &Headers;
}

impl<'a,'b> Request for HyperRequest<'a,'b> {
    fn headers(&self) -> &Headers {
        &self.headers
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
                  Content-Length: 217\r\n\
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
            Ok((fields, files)) => {
                assert_eq!(fields.len(), 1);
                for (key, val) in fields {
                    if &key == "field1" {
                        assert_eq!(&val, "data1");
                    }
                }

                assert_eq!(files.len(), 2);
                for (key, file) in files {
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
}
