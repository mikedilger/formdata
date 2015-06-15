
use std::io;
use std::fmt::{self,Display};
use std::string::FromUtf8Error;
use std::error::Error as StdError;

use super::httparse;
use super::hyper;

/// An error type for the `formdata` crate
#[derive(Debug)]
pub enum Error {
    /// The hyper request did not have a content-type header
    NoRequestContentType,
    /// The hyper request content-type top-level Mime was not `Multipart`
    NotMultipart,
    /// The hyper request content-type sub-level Mime was not `FormData`
    NotFormData,
    /// The content-type header failed to specify boundary token
    BoundaryNotSpecified,
    /// A multipart section contained only partial headers
    PartialHeaders,
    /// A multipart section did not have the required content-disposition header
    MissingDisposition,
    /// A multipart section content-disposition header failed to specify a name
    NoName,
    /// The request body ended prior to reaching the expected terminating boundary
    Eof,
    /// A parse error occurred while parsing the headers of a multipart section
    Httparse(httparse::Error),
    /// An I/O error occurred
    Io(io::Error),
    /// An error was returned from hyper
    Hyper(hyper::Error),
    /// An error occurred during UTF-8 processing
    Utf8(FromUtf8Error),
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

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result
    {
        match *self {
            Error::Httparse(ref e) =>
                format!("{}: {:?}", self.description(), e).fmt(f),
            Error::Io(ref e) =>
                format!("{}: {}", self.description(), e).fmt(f),
            Error::Hyper(ref e) =>
                format!("{}: {}", self.description(), e).fmt(f),
            Error::Utf8(ref e) =>
                format!("{}: {}", self.description(), e).fmt(f),
            _ => format!("{}", self.description()).fmt(f),
        }
    }
}

impl StdError for Error {
    fn description(&self) -> &str
    {
        match *self {
            Error::NoRequestContentType => "The hyper request did not have a content-type header",
            Error::NotMultipart => "The hyper request content-type top-level Mime was not multipart",
            Error::NotFormData => "The hyper request content-type sub-level Mime was not form-data",
            Error::BoundaryNotSpecified => "The content-type header failed to specify a boundary token",
            Error::PartialHeaders => "A multipart section contained only partial headers",
            Error::MissingDisposition => "A multipart section did not have the required content-disposition header",
            Error::NoName => "A multipart section content-disposition header failed to specify a name",
            Error::Eof => "The request body ended prior to reaching the expected terminating boundary",
            Error::Httparse(_) => "A parse error occurred while parsing the headers of a multipart section",
            Error::Io(_) => "I/O error",
            Error::Hyper(_) => "Hyper error",
            Error::Utf8(_) => "UTF-8 error",
        }
    }
}
