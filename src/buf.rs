use std::io::{BufRead, ErrorKind, Result, Write};

/// Extends any type that implements BufRead with a stream_until_token() function.
pub trait BufReadExt: BufRead {
    /// Streams all bytes to `out` until the `token` delimiter is reached.
    ///
    /// This function will continue to read (and stream) bytes from the underlying stream until the
    /// token or end-of-file byte is found. Once found, all bytes up to the token (if found) will
    /// have been streamed to `out` and this input stream will advance past the token (the token
    /// will be discarded).
    ///
    /// This function will return `Ok(n)` where `n` is the number of bytes which were read,
    /// including the token if it was found. If the token was not found, it will still return
    /// `Ok(n)`.
    ///
    /// # Errors
    ///
    /// This function will ignore all instances of `ErrorKind::Interrupted` and will otherwise
    /// return any errors returned by `fill_buf`.
    fn stream_until_token<W: Write>(&mut self, token: &[u8], out: &mut W) -> Result<usize> {
        stream_until_token(self, token, out)
    }
}

// Implement BufReadExt for everything that implements BufRead.
impl<T: BufRead> BufReadExt for T { }

fn stream_until_token<R: BufRead + ?Sized, W: Write>(stream: &mut R, token: &[u8], mut out: &mut W)
                                                     -> Result<usize> {
    let mut read = 0;
    // Represents the size of a token prefix found at the end of a buffer, usually 0. If not 0, the
    // beginning of the next buffer is checked for the suffix to find matches that straddle two
    // buffers.
    let mut partial: usize = 0;

    loop {
        let (found, used) = {
            let buffer = match stream.fill_buf() {
                Ok(n) => n,
                Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => return Err(err)
            };

            // If last buffer ended in a token prefix, check if this one starts with the matching
            // suffix.
            if partial > 0 && buffer[..token.len() - partial] == token[partial..] {
                (true, token.len() - partial)
            } else {
                if partial > 0 {
                    // Last buffer ended in a token prefix, but it didn't pan out, so we need to
                    // push it along.
                    try!( out.write_all(&token[..partial]) );
                }

                let index = buffer
                    .windows(token.len())
                    .enumerate()
                    .filter(|&(_, t)| t == token)
                    .map(|(i, _)| i)
                    .next();

                // Search for the token.
                match index {
                    Some(index) => {
                        try!(out.write_all(&buffer[..index]));
                        (true, index + token.len())
                    },
                    None => {
                        // Check for partial matches at the end of the buffer
                        let mut window = token.len() - 1;
                        if buffer.len() < window {
                            window = buffer.len();
                        }

                        partial = (1..window+1)
                            .rev()
                            .filter(|&w| token[..w] == buffer[buffer.len() - w..])
                            .next()
                            .unwrap_or(0);

                        // Push all except the partial token at the end (if any)
                        try!(out.write_all(&buffer[..buffer.len()-partial]));
                        // Mark it all as consumed.
                        (false, buffer.len())
                    }
                }
            }
        };

        stream.consume(used);
        read += used;

        if found || used == 0 {
            break;
        }
    }

    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufReader, Cursor};

    #[test]
    fn stream_until_token() {
        let mut buf = Cursor::new(&b"123456"[..]);
        let mut result: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"78", &mut result).unwrap(), 6);
        assert_eq!(result, b"123456");
        let mut buf = Cursor::new(&b"12345678"[..]);
        let mut result: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"34", &mut result).unwrap(), 4);
        assert_eq!(result, b"12");
        result.truncate(0);
        assert_eq!(buf.stream_until_token(b"78", &mut result).unwrap(), 4);
        assert_eq!(result, b"56");

        let mut buf = Cursor::new(&b"bananas for nana"[..]);
        let mut result: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"nan", &mut result).unwrap(), 5);
        assert_eq!(result, b"ba");
        result.truncate(0);
        assert_eq!(buf.stream_until_token(b"nan", &mut result).unwrap(), 10);
        assert_eq!(result, b"as for ");
        result.truncate(0);
        assert_eq!(buf.stream_until_token(b"nan", &mut result).unwrap(), 1);
        assert_eq!(result, b"a");
        result.truncate(0);
        assert_eq!(buf.stream_until_token(b"nan", &mut result).unwrap(), 0);
        assert_eq!(result, b"");
    }

    #[test]
    fn stream_until_token_straddle_test() {
        let cursor = Cursor::new(&b"12345TOKEN345678"[..]);
        let mut buf = BufReader::with_capacity(8, cursor);
        let mut result: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"TOKEN", &mut result).unwrap(), 10);
        assert_eq!(result, b"12345");
        result.truncate(0);
        assert_eq!(buf.stream_until_token(b"TOKEN", &mut result).unwrap(), 6);
        assert_eq!(result, b"345678");
        result.truncate(0);
        assert_eq!(buf.stream_until_token(b"TOKEN", &mut result).unwrap(), 0);
        assert_eq!(result, b"");

        let cursor = Cursor::new(&b"12345TOKE23456781TOKEN78"[..]);
        let mut buf = BufReader::with_capacity(8, cursor);
        let mut result: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"TOKEN", &mut result).unwrap(), 22);
        assert_eq!(result, b"12345TOKE23456781");
    }

    // This tests against github issue #1
    #[test]
    fn stream_until_token_large_token_test() {
        let cursor = Cursor::new(&b"IAMALARGETOKEN7812345678"[..]);
        let mut buf = BufReader::with_capacity(8, cursor);
        let mut v: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"IAMALARGETOKEN", &mut v).unwrap(), 14);
        assert_eq!(v, b"");
        assert_eq!(buf.stream_until_token(b"IAMALARGETOKEN", &mut v).unwrap(), 10);
        assert_eq!(v, b"7812345678");

        let cursor = Cursor::new(&b"0IAMALARGERTOKEN12345678"[..]);
        let mut buf = BufReader::with_capacity(8, cursor);
        let mut v: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"IAMALARGERTOKEN", &mut v).unwrap(), 16);
        assert_eq!(v, b"0");
        v.truncate(0);
        assert_eq!(buf.stream_until_token(b"IAMALARGERTOKEN", &mut v).unwrap(), 8);
        assert_eq!(v, b"12345678");
    }

    // This tests against github issue #11
    #[test]
    fn stream_until_token_double_straddle_test() {
        let cursor = Cursor::new(&b"12345IAMALARGETOKEN4567"[..]);
        let mut buf = BufReader::with_capacity(8, cursor);
        let mut v: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"IAMALARGETOKEN", &mut v).unwrap(), 5+14);
        assert_eq!(v, b"12345");
        v.truncate(0);
        assert_eq!(buf.stream_until_token(b"IAMALARGETOKEN", &mut v).unwrap(), 4);
        assert_eq!(v, b"4567");
    }

    // This tests against github issue #12
    #[test]
    fn stream_until_token_multiple_prefix_text() {
        let cursor = Cursor::new(&b"12barbarian4567"[..]);
        let mut buf = BufReader::with_capacity(8, cursor);
        let mut v: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"barbarian", &mut v).unwrap(), 2+9);
        assert_eq!(v, b"12");

        let cursor = Cursor::new(&b"12barbarbarian7812"[..]);
        let mut buf = BufReader::with_capacity(8, cursor);
        let mut v: Vec<u8> = Vec::new();
        assert_eq!(buf.stream_until_token(b"barbarian", &mut v).unwrap(), 5+9);
        assert_eq!(v, b"12bar");
    }
}
