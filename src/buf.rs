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
    // Represents the sizes of possible token prefixes found at the end of the last buffer, usually
    // empty. If not empty, the beginning of this buffer is checked for the matching suffixes to
    // to find tokens that straddle two buffers. Entries should be in longest prefix to shortest
    // prefix order.
    let mut prefix_lengths: Vec<usize> = Vec::new();

    'stream:
    loop {
        let mut found = false;
        let mut used: usize = 0;

        // This is not actually meant to repeat, we only need the break functionality of a loop.
        // The reader is encouraged to try their hand at coding this better, noting that buffer must
        // drop out of scope before stream can be used again.
        let mut do_once = true;
        'buffer:
        while do_once {
            do_once = false;

            // Fill the buffer (without consuming)
            let buffer = match stream.fill_buf() {
                Ok(n) => n,
                Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => return Err(err)
            };
            if buffer.len() == 0 {
                break 'stream;
            }

            // If the buffer starts with a token suffix matching a token prefix from the end of the
            // previous buffer, then we have found a token.
            if prefix_lengths.len() > 0 {
                let largest_prefix_len = prefix_lengths[0];

                // FIXME: once Vec::drain() stabilizes, use that instead
                let drain = prefix_lengths.clone();
                prefix_lengths.truncate(0);

                let mut partmatch = false;
                for &prefix_len in drain.iter() {
                    // If the buffer is too small to fit an entire suffix
                    if buffer.len() < token.len() - prefix_len {
                        if buffer[..] == token[prefix_len..prefix_len + buffer.len()] {
                            // that prefix just got bigger and needs to be preserved
                            prefix_lengths.push(prefix_len + buffer.len());
                            partmatch = true;
                        }
                    } else {
                        if buffer[..token.len() - prefix_len] == token[prefix_len..] {
                            try!(out.write_all(&token[..largest_prefix_len - prefix_len]));
                            found = true;
                            used = token.len() - prefix_len;
                            break 'buffer;
                        }
                    }
                }

                if ! partmatch {
                    // No prefix matched, so we should write the largest prefix length, since we
                    // didn't write it when we first saw it
                    try!(out.write_all(&token[..largest_prefix_len]));
                }
            }

            // Get the index index of the first token in the middle of the buffer, if any
            let index = buffer
                .windows(token.len())
                .enumerate()
                .filter(|&(_, t)| t == token)
                .map(|(i, _)| i)
                .next();

            if let Some(index) = index {
                try!(out.write_all(&buffer[..index]));
                found = true;
                used = index + token.len();
                break 'buffer;
            }

            // Check for token prefixes at the end of the buffer.
            let mut window = token.len() - 1;
            if buffer.len() < window {
                window = buffer.len();
            }
            // Remember the largest prefix for writing later if it didn't match
            let mut reserve = if prefix_lengths.len() > 0 {
                buffer.len()
            } else {
                0
            };
            for prefix in (1..window+1).rev()
                .filter(|&w| token[..w] == buffer[buffer.len() - w..])
            {
                if reserve == 0 {
                    reserve = prefix;
                }
                prefix_lengths.push(prefix)
            }

            try!(out.write_all(&buffer[..buffer.len()-reserve]));
            used = buffer.len();
        }

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
