// # References
//
// "Returning Values from Forms: multipart/form-data" https://www.ietf.org/rfc/rfc2388.txt
// "The Content-Disposition Header Field" https://www.ietf.org/rfc/rfc2183.txt

use std::ascii::AsciiExt;
use std::fmt;

use hyper::error::Error as HyperError;
use hyper::header::{Header, HeaderFormat, parsing};

/// A `Content-Disposition` header for only `multipart/*`, which is found within the MIME multipart
/// sections.
///
/// This is an internal type not meant for general public usage, as its implementation limited in
/// scope.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentDisposition {
    pub disposition: String,
    pub name: Option<String>,
    pub filename: Option<String>,
}

impl Header for ContentDisposition {
    fn header_name() -> &'static str {
        "Content-Disposition"
    }

    fn parse_header(raw: &[Vec<u8>]) -> Result<ContentDisposition, HyperError> {
        parsing::from_one_raw_str(raw).and_then(|s: String| {
            let mut sections = s.split(';');
            let disposition = match sections.next() {
                Some(s) => s.trim().to_ascii_lowercase(),
                None => return Err(HyperError::Header),
            };

            let mut cd = ContentDisposition {
                disposition: disposition,
                name: None,
                filename: None
            };

            for section in sections {
                let mut parts = section.split('=');

                let key = if let Some(key) = parts.next() {
                    key.trim().to_ascii_lowercase()
                } else {
                    continue;
                };

                let mut val = if let Some(val) = parts.next() {
                    val.trim()
                } else {
                    continue;
                };

                if val.chars().next() == Some('"') && val.chars().rev().next() == Some('"') {
                    // Unwrap the quotation marks.
                    val = &val[1..val.len() - 1];
                }

                match &*key {
                    "name" => cd.name = Some(val.to_string()),
                    "filename" => cd.filename = Some(val.to_string()),
                    _ => { },
                }
            }

            Ok(cd)
        })
    }
}

impl HeaderFormat for ContentDisposition {
    fn fmt_header(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("form-data")
    }
}

#[cfg(test)]
mod tests {
    use super::ContentDisposition;
    use hyper::header::Header;

    #[test]
    fn parse_header() {
        let a = [b"form-data; dummy=3; name=upload;\r\n filename=\"sample.png\"".to_vec()];
        let a: ContentDisposition = ContentDisposition::parse_header(a.as_ref()).unwrap();
        let b = ContentDisposition {
            disposition: String::from("form-data"),
            name: Some("upload".to_string()),
            filename: Some("sample.png".to_string()),
        };
        assert_eq!(a, b);

        assert!(ContentDisposition::parse_header([b"".to_vec()].as_ref()).is_err());
    }
}
