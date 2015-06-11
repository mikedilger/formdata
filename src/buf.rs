
use std::io::{BufRead,ErrorKind,Result};

/// This trait extends any type that implements BufRead with a
/// read_until_token() function.
pub trait BufReadPlus: BufRead {
    /// Read all bytes until the `token` delimiter is reached.
    ///
    /// This function will continue to read (and buffer) bytes from the
    /// underlying stream until the token or EOF is found. Once found, all
    /// bytes up to the token (if found) will be appended to `buf`, and
    /// the stream will advance past the token (the token will be discarded).
    ///
    /// This function will return `Ok(n)` where `n` is the number of bytes
    /// which were read, counting the token if it was found.
    ///
    /// # Errors
    ///
    /// This function will ignore all instances of `ErrorKind::Interrupted` and
    /// will otherwise return any errors returned by `fill_buf`.
    ///
    /// If an I/O error is encountered then all bytes read so far will be
    /// present in `buf` and its length will have been adjusted appropriately.
    fn read_until_token(&mut self, token: &[u8], buf: &mut Vec<u8>) -> Result<usize> {
        read_until_token(self, token, buf)
    }
}

use std::io::{Read,Write};
use std::io::{BufReader,BufStream,Cursor,Empty,StdinLock,Take};
impl<R: Read> BufReadPlus for BufReader<R> {}
impl<S: Read + Write> BufReadPlus for BufStream<S> {}
impl<'a> BufReadPlus for Cursor<&'a [u8]> {}
impl<'a> BufReadPlus for Cursor<&'a mut [u8]> {}
impl<'a> BufReadPlus for Cursor<Vec<u8>> {}
impl<'a, B: BufRead + ?Sized> BufReadPlus for &'a mut B {}
impl<B: BufRead + ?Sized> BufReadPlus for Box<B> {}
impl<'a> BufReadPlus for &'a [u8] {}
impl BufReadPlus for Empty {}
impl<'a> BufReadPlus for StdinLock<'a> {}
impl<T: BufRead> BufReadPlus for Take<T> {}

fn read_until_token<R: BufRead + ?Sized>(r: &mut R, token: &[u8], buf: &mut Vec<u8>)
                                         -> Result<usize> {
    let mut read = 0;
    let mut partial: Option<usize> = None;
    loop {
        let (done,used) = {
            let available = match r.fill_buf() {
                Ok(n) => n,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e)
            };
            // Check for 2nd half of partial match straddling filled available buffer
            if partial.is_some() &&
                available[..token.len() - partial.unwrap()] == token[partial.unwrap()..]
            {
                let trunc = buf.len() - partial.unwrap();
                buf.truncate(trunc);
                (true, token.len() - partial.unwrap())
            }
            else {
                let mut found_at: Option<usize> = None;
                for (i,w) in available.windows(token.len()).enumerate() {
                    if w == token {
                        found_at = Some(i);
                        break;
                    }
                }
                match found_at {
                    Some(i) => {
                        buf.push_all(&available[..i]);
                        (true, i + token.len())
                    },
                    None => {
                        // Check for partial matches at the end of the buffer
                        let mut width = token.len() - 1;
                        if available.len() > width {
                            while width>0 {
                                if token[..width] == available[available.len() - width..] {
                                    partial = Some(width);
                                    break;
                                }
                                width = width - 1;
                            }
                        }
                        buf.push_all(available);
                        (false, available.len())
                    }
                }
            }
        };
        r.consume(used);
        read += used;
        if done || used == 0 {
            return Ok(read);
        }
    }
}


#[cfg(test)]
mod tests {
    use std::io::{Cursor,BufReader};
    use super::BufReadPlus;

    #[test]
    fn read_until_token() {
        let mut buf = Cursor::new(&b"123456"[..]);
        let mut v = Vec::new();
        assert_eq!(buf.read_until_token(b"78", &mut v).unwrap(), 6);
        assert_eq!(v, b"123456");

        let mut buf = Cursor::new(&b"12345678"[..]);
        let mut v = Vec::new();
        assert_eq!(buf.read_until_token(b"34", &mut v).unwrap(), 4);
        assert_eq!(v, b"12");
        v.truncate(0);
        assert_eq!(buf.read_until_token(b"78", &mut v).unwrap(), 4);
        assert_eq!(v, b"56");

        let mut buf = Cursor::new(&b"bananas for nana"[..]);
        let mut v = Vec::new();
        assert_eq!(buf.read_until_token(b"nan", &mut v).unwrap(), 5);
        assert_eq!(v, b"ba");
        v.truncate(0);
        assert_eq!(buf.read_until_token(b"nan", &mut v).unwrap(), 10);
        assert_eq!(v, b"as for ");
        v.truncate(0);
        assert_eq!(buf.read_until_token(b"nan", &mut v).unwrap(), 1);
        assert_eq!(v, b"a");
        v.truncate(0);
        assert_eq!(buf.read_until_token(b"nan", &mut v).unwrap(), 0);
        assert_eq!(v, b"");
    }

    #[test]
    fn read_until_token_straddle_test() {
        let cursor = Cursor::new(&b"12345TOKEN345678"[..]);
        let mut buf = BufReader::with_capacity(8, cursor);
        let mut v = Vec::new();
        assert_eq!(buf.read_until_token(b"TOKEN", &mut v).unwrap(), 10);
        assert_eq!(v, b"12345");
        v.truncate(0);
        assert_eq!(buf.read_until_token(b"TOKEN", &mut v).unwrap(), 6);
        assert_eq!(v, b"345678");
        v.truncate(0);
        assert_eq!(buf.read_until_token(b"TOKEN", &mut v).unwrap(), 0);
        assert_eq!(v, b"");
    }
}
