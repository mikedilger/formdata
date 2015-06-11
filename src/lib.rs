//! This crate parses and processes hyper::server::Request data
//! containing multipart/form-data formatted data in a streaming
//! fashion.

// FIXME: replace push_all() with extend()
#![feature(buf_stream,collections)]

extern crate collections;
extern crate hyper;
extern crate mime;
extern crate httparse;
extern crate libc;

#[cfg(test)]
mod mock;
pub mod buf;
mod content_disposition;
pub mod error;

pub use error::Error;
pub use buf::BufReadPlus;

//use std::path::PathBuf;
use std::io::{BufReader,BufRead};

use hyper::server::Request;
use hyper::header::{Headers,ContentType};
use mime::{Mime,TopLevel,SubLevel,Attr,Value};

pub use content_disposition::ContentDispositionFormData;

#[cfg(test)]
use std::net::SocketAddr;
//#[cfg(test)]
//use std::io::Read;

// FIXME: define these
pub struct UploadedFile;
/*{
    path: PathBuf,
    name: Option<String>,
    content_type: Mime,
}*/

pub fn parse_multipart<'a,'b>(
    request: &mut Request<'a,'b>)
    -> Result< (Vec<(String,String)>, Vec<UploadedFile>), Error >
{
    let mut parameters: Vec<(String,String)> = Vec::new();
    let mut files: Vec<UploadedFile> = Vec::new();

    let string_boundary = try!( get_boundary(request) );
    println!("Boundary is {}", string_boundary);
    let boundary: Vec<u8> = string_boundary.into_bytes();

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
                let read = try!( r.read_until_token( &*boundary, &mut buf ) );
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
                let read = try!( r.read_until_token( b"\r\n", &mut buf ) );
                if read==0 { return Err(Error::Eof); }

                buf.truncate(0);
                let read = try!( r.read_until_token( b"\r\n\r\n", &mut buf ) );
                if read==0 { return Err(Error::Eof); }
                buf.push_all(b"\r\n\r\n"); // parse_headers() needs this token at the end

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
            // FIXME:  CapturingFile should be done properly
            State::CapturingValue | State::CapturingFile => {
                buf.truncate(0);
                let read = try!( r.read_until_token( &*boundary, &mut buf ) );
                if read==0 { return Err(Error::Eof); }

                let cd: &ContentDispositionFormData = headers.get().unwrap();

                let key = match cd.name {
                    None => return Err(Error::NoName),
                    Some(ref name) => name.clone()
                };
                let value = try!( String::from_utf8(buf) );

                parameters.push( (key,value) );

                state = State::ReadingHeaders;
            },
            /*
            State::CapturingFile => {
                // FIXME: default to text/plain if no content-type header.
                // FIXME: handle content-transfer-encoding
                // FIXME: handle content-type: multipart/mixed as multiple files
                return Err(Error::NotImplementedYet);
            },
            */
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
            for (k,v) in fields {
                println!("{}={}",k,v);
            }
            assert!(true);
        }
        Err(e) => {
            println!("FAILED ON: {:?}",e);
            assert!(false);
        }
    }
}
