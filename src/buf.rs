
use std::io::{BufRead,ErrorKind,Result,Write};

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
    /// which were read, counting the token if it was found.  If the token was
    /// not found, it will still return `Ok(n)`
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

use std::io::{Read};
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
    // Partial represents the size of a token prefix found at the end of a buffer,
    // usually 0.  If not zero, the beginning of the next buffer is checked for the
    // suffix to find matches that straddle two buffers
    let mut partial: usize = 0;
    loop {
        let (done,used) = {
            let available = match r.fill_buf() {
                Ok(n) => n,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e)
            };
            // If last buffer ended in a token prefix, check if this one starts with the
            // matching suffix
            if partial > 0 && available[..token.len() - partial] == token[partial..] {
                (true, token.len() - partial)
            }
            else {
                if partial > 0 {
                    // Last buffer ended in a token prefix, but it didn't pan out, so
                    // we need to push it along
                    buf.push_all(&token[..partial])
                }
                // Search for the token
                match available
                    .windows(token.len())
                    .enumerate()
                    .filter(|&(_,w)| { w == token })
                    .map(|(i,_)| { i })
                    .next()
                {
                    Some(i) => {
                        buf.push_all(&available[..i]);
                        (true, i + token.len())
                    },
                    None => {
                        // Check for partial matches at the end of the buffer
                        partial = if available.len() > token.len() - 1 {
                            match (1..(token.len()-1))
                                .rev()
                                .filter(|&width| {
                                    token[..width] == available[available.len() - width..]
                                })
                                .next()
                            {
                                Some(width) => width,
                                None => 0
                            }
                        } else { 0 };
                        // Push all except the partial token at the end (if any)
                        buf.push_all(&available[..available.len()-partial]);
                        (false, available.len()) // But mark it all consumed
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

        let cursor = Cursor::new(&b"12345TOKE23456781TOKEN78"[..]);
        let mut buf = BufReader::with_capacity(8, cursor);
        let mut v = Vec::new();
        assert_eq!(buf.read_until_token(b"TOKEN", &mut v).unwrap(), 22);
        assert_eq!(v, b"12345TOKE23456781");
    }
}
