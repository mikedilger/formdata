//! This crate parses and processes `hyper::server::Request` data
//! that contains `multipart/form-data` content.
//!
//! The main entry point is `parse_multipart`

extern crate hyper;
#[macro_use] extern crate mime;
extern crate httparse;
extern crate libc;
extern crate tempdir;
extern crate textnonce;

#[cfg(test)]
mod mock;
pub mod buf;
mod content_disposition;
pub mod error;

pub use error::Error;
pub use buf::BufReadPlus;

use std::path::PathBuf;
use std::fs::File;
use std::io::{BufReader,BufRead};

use hyper::server::Request;
use hyper::header::{Headers,ContentType};
use mime::{Mime,TopLevel,SubLevel,Attr,Value};
use tempdir::TempDir;
use textnonce::TextNonce;

pub use content_disposition::ContentDispositionFormData;

#[cfg(test)]
use std::net::SocketAddr;

/// This structure represents uploaded files which were received as
/// part of the `multipart/form-data` parsing.  They are streamed to
/// disk because they may not fit in memory.
pub struct UploadedFile {
    /// This is the temporary file where the data was saved.
    pub path: PathBuf,
    /// This is the filename that was specified in the data, unfiltered.  It may or may not
    /// be legal on the local filesystem.
    pub filename: Option<String>,
    /// This is the content-type that was specified in the data, unvalidated.
    pub content_type: Mime,
    /// This is the actual size of the file received
    pub size: usize,
}

/// This function parses and processes `hyper::server::Request` data
/// that contains `multipart/form-data` content.  It does this in a streaming
/// fashion, saving embedded uploaded files to disk as they are encountered by
/// the parser.
///
/// It returns two sets of data.  The first, `Vec<(String,String)>`, are the
/// variable-value pairs from the POST-ed form.  The second
/// `Vec<(String,UploadedFile)>` are the variable-file pairs from the POST-ed
/// form, where `UploadedFile` structures describe the uploaded file.
pub fn parse_multipart<'a,'b>(
    request: &mut Request<'a,'b>)
    -> Result< (Vec<(String,String)>, Vec<(String,UploadedFile)>), Error >
{
    let mut parameters: Vec<(String,String)> = Vec::new();
    let mut files: Vec<(String,UploadedFile)> = Vec::new();

    let string_boundary = try!( get_boundary(request) );
    println!("Boundary is {}", string_boundary);
    let boundary: Vec<u8> = string_boundary.into_bytes();
    let mut crlf_boundary: Vec<u8> = Vec::with_capacity(2 + boundary.len());
    crlf_boundary.extend(b"\r\n".iter().map(|&i| i));
    crlf_boundary.extend(boundary.clone());

    // Request implements Read.  Internally it's a BufReader, but it's not
    // exposed that way.  We need to wrap it into a BufReader to get that
    // functionality ourselves.
    let mut r = BufReader::with_capacity(4096,request);

    enum State {
        Discarding, // Discard until after initial boundary and CRLF
        CapturingValue, // Capture value until boundary, then discard past CRLF
        CapturingFile, // Capture file until boundary, then discard past CRLF
        ReadingHeaders, // Capture headers to blank line
    }

    let mut state = State::Discarding;
    let mut headers = Headers::new();

    loop {
        let mut buf: Vec<u8> = Vec::new();

        match state {
            State::Discarding => {
                // Read up to and including the boundary
                let read = try!( r.stream_until_token( &*boundary, &mut buf ) );
                if read==0 { return Err(Error::Eof); }

                state = State::ReadingHeaders;
            },
            State::ReadingHeaders => {
                {
                    // IF the next two characters are '--' (look ahead), then we are finished
                    let peeker = try!( r.fill_buf() );
                    if peeker.len() >= 2 && &peeker[..2]==b"--" {
                        return Ok((parameters, files))
                    }
                } // drop peeker

                // Read up to and including the CRLF after the boundary
                let read = try!( r.stream_until_token( b"\r\n", &mut buf ) );
                if read==0 { return Err(Error::Eof); }

                buf.truncate(0);
                let read = try!( r.stream_until_token( b"\r\n\r\n", &mut buf ) );
                if read==0 { return Err(Error::Eof); }
                buf.extend(b"\r\n\r\n".iter().map(|&i| i)); // parse_headers() needs this token at the end

                // Parse the headers
                let mut header_memory = [httparse::EMPTY_HEADER; 4];
                match httparse::parse_headers( &*buf, &mut header_memory) {
                    Ok(httparse::Status::Complete((_,raw_headers))) => {
                        // Turn raw headers into hyper headers
                        headers = try!( Headers::from_raw(raw_headers) );

                        let cd: &ContentDispositionFormData = match headers.get() {
                            Some(x) => x,
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
                        println!("Header Parsing was Partial");
                        return Err(Error::ParseError);
                    },
                    Err(e) => return Err( From::from(e) ),
                }
            },
            State::CapturingValue => {
                buf.truncate(0);
                let _ = try!( r.stream_until_token( &*crlf_boundary, &mut buf ) );

                let cd: &ContentDispositionFormData = headers.get().unwrap();

                let key = match cd.name {
                    None => return Err(Error::NoName),
                    Some(ref name) => name.clone()
                };
                let value = try!( String::from_utf8(buf) );

                parameters.push( (key,value) );

                state = State::ReadingHeaders;
            },
            State::CapturingFile => {
                // Setup a file to capture the contents
                let mut temp = try!( TempDir::new("formdata") ).into_path();
                temp.push( TextNonce::sized_urlsafe(32).unwrap().into_string() );
                let mut tmpfile = try!( File::create(temp.clone()) );

                // Stream out the file
                let read = try!( r.stream_until_token( &*crlf_boundary, &mut tmpfile ) );

                let cd: &ContentDispositionFormData = headers.get().unwrap();
                let key = match cd.name {
                    None => return Err(Error::NoName),
                    Some(ref name) => name.clone()
                };
                // FIXME: handle content-type header, and default to text/plain if
                //        no content-type header.
                // FIXME: handle content-type: multipart/mixed as multiple files
                // FIXME: handle content-transfer-encoding

                let ufile = UploadedFile {
                    path: temp,
                    filename: cd.filename.clone(),
                    content_type: mime!(Text/Plain; Charset=Utf8), // FIXME
                    size: read - crlf_boundary.len()
                };
                files.push( (key,ufile) );

                state = State::ReadingHeaders;
            },
        }
    }
}

fn get_boundary<'a,'b>(request: &Request<'a,'b>) -> Result<String,Error>
{
    // Verify that the request is 'content-type: multipart/form-data'
    let content_type: &ContentType = match request.headers.get() {
        Some(h) => h,
        None => return Err(Error::NoRequestContentType),
    };
    let ContentType(ref mime) = *content_type;
    let Mime(ref top_level, ref sub_level, ref params) = *mime;
    if *top_level != TopLevel::Multipart {
        return Err(Error::NotMultipart);
    }
    if *sub_level != SubLevel::FormData {
        return Err(Error::NotFormData);
    }

    // Get the boundary token
    for &(ref attr,ref value) in params.iter() {
        match (attr,value) {
            (&Attr::Ext(ref k), &Value::Ext(ref v)) => {
                if *k=="boundary" {
                    return Ok( format!("--{}",v.clone()) )
                }
            }
            _ => {}
        }
    }
    Err(Error::BoundaryNotSpecified)

}

#[test]
fn test1() {
    use mock::MockStream;
    use hyper::net::NetworkStream;
    use hyper::buffer::BufReader;

    let input=b"POST / HTTP/1.1\r\n\
                Host: example.domain\r\n\
                Content-Type: multipart/form-data; boundary=\"abcdefg\"\r\n\
                Content-Length: 217\r\n\
                \r\n\
                --abcdefg\r\n\
                Content-Disposition: form-data; name=\"field1\"\r\n\
                \r\n\
                data1\r\n\
                --abcdefg\r\n\
                Content-Disposition: form-data; name=\"field2\"; filename=\"file.txt\"\r\n\
                Content-Type: text/plain\r\n\
                \r\n\
                This is a file\r\n\
                with two lines\r\n\
                --abcdefg--";
    let mut mock = MockStream::with_input(input);

    let mock: &mut NetworkStream = &mut mock;
    let mut stream = BufReader::new(mock);
    let sock: SocketAddr = "127.0.0.1:80".parse().unwrap();

    let mut req = Request::new(&mut stream, sock).unwrap();

    match parse_multipart(&mut req) {
        Ok((fields,files)) => {
            assert_eq!(fields.len(),1);
            for (k,v) in fields {
                if &k=="field1" { assert_eq!( &v, "data1" ) }
            }
            assert_eq!(files.len(),1);
            for (k,file) in files {
                if &k=="field2" {
                    assert_eq!(file.size, 30);
                    assert!(file.filename.is_some());
                    assert_eq!(&file.filename.unwrap(), "file.txt");
                    // FIXME add content_type check after impl
                }
            }

        }
        Err(e) => {
            println!("FAILED ON: {:?}",e);
            assert!(false);
        }
    }
}
