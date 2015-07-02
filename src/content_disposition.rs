// Refer:
// https://www.ietf.org/rfc/rfc2388.txt:  Returning Values from Forms:  multipart/form-data
// https://www.ietf.org/rfc/rfc2183.txt:  The Content-Disposition Header Field

use std::fmt;
use std::ascii::AsciiExt;

use hyper::header::{Header, HeaderFormat, parsing};
use hyper::error::Error as HyperError;

/// We define a Content-Disposition form-data only header, which is found within
/// the mime multipart sections.  NOT FOR GENERAL USE
#[derive(Debug,Clone,PartialEq)]
pub struct ContentDispositionFormData {
    pub name: Option<String>,
    pub filename: Option<String>,
}

impl Header for ContentDispositionFormData {
    fn header_name() -> &'static str {
        "Content-Disposition"
    }

    fn parse_header(raw: &[Vec<u8>]) -> Result<ContentDispositionFormData,HyperError> {
        parsing::from_one_raw_str(raw).and_then(|s: String| {
            let mut sections = s.split(';');
            match sections.next() {
                None => return Err(HyperError::Header),
                Some(s) => {
                    if &s.trim().to_ascii_lowercase() != "form-data" {
                        return Err(HyperError::Header)
                    }
                }
            };
            let mut cd = ContentDispositionFormData { name: None, filename: None };
            for section in sections {
                let mut parts = section.split('=');
                if parts.clone().count() != 2 { continue; }
                let key = parts.next().unwrap().trim().to_ascii_lowercase();
                let mut val = parts.next().unwrap().trim();
                if val.chars().next()==Some('"') && val.chars().rev().next()==Some('"') {
                    val = &val[1..val.len()-1]; // unwrap quotes
                }
                match &*key {
                    "name" => cd.name = Some(val.to_string()),
                    "filename" => cd.filename = Some(val.to_string()),
                    _ => {}
                }
            }
            Ok(cd)
        })
    }
}

impl HeaderFormat for ContentDispositionFormData {
    fn fmt_header(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("form-data")
    }
}

#[cfg(test)]
mod tests {
    use super::ContentDispositionFormData;
    use hyper::header::Header;
    use hyper::error::Error as HyperError;

    #[test]
    fn parse_header() {
        let a: ContentDispositionFormData =
            ContentDispositionFormData::parse_header(
                [b"form-data; dummy=3; name=upload;\r\n filename=\"sample.png\"".to_vec()].as_ref() )
            .unwrap();
        let b = ContentDispositionFormData {
            name: Some("upload".to_string()),
            filename: Some("sample.png".to_string()),
        };
        assert_eq!(a, b);

        match ContentDispositionFormData::parse_header([b"".to_vec()].as_ref()) {
            Ok(_) => assert!(false, "Empty header was parsed as ok!"),
            Err(HyperError::Header) => assert!(true),
            Err(_) => assert!(false, "Empty header parsed with wrong error"),
        }
    }

    // RFC 2231 (obsoletes 2184) is NOT supported presently
    // See closed issue #3
    //
    //#[test]
    //fn parse_header_test_rfc_2231() {
    //    let a: ContentDispositionFormData =
    //        ContentDispositionFormData::parse_header(
    //            [b"form-data; dummy=3;\r\n filename*0=\"sample%20music\"; filename*1=\".png\"".to_vec()].as_ref() )
    //        .unwrap();
    //    let b = ContentDispositionFormData {
    //        name: None,
    //        filename: Some("sample music.png".to_string()),
    //    };
    //    assert_eq!(a, b);
    //}
}
