
use std::io;
use std::string::FromUtf8Error;
use super::httparse;
use super::hyper;

/// An error type for the `formdata` crate
#[derive(Debug)]
pub enum Error {
    NoRequestContentType,
    NotMultipart,
    NotFormData,
    NotImplementedYet,
    Eof,
    Io(io::Error),
    Httparse(httparse::Error),
    ParseError,
    MissingContentLength,
    Hyper(hyper::Error),
    MissingDisposition,
    NoName,
    Utf8(FromUtf8Error),
    BoundaryNotSpecified
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {  Error::Io(err)  }
}
impl From<httparse::Error> for Error {
    fn from(err: httparse::Error) -> Error {  Error::Httparse(err)  }
}
impl From<hyper::Error> for Error {
    fn from(err: hyper::Error) -> Error { Error::Hyper(err) }
}
impl From<FromUtf8Error> for Error {
    fn from(err: FromUtf8Error) -> Error { Error::Utf8(err) }
}
