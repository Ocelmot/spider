use crate::{error::ErrorKind, LinkResult};

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

const TAIL_CHARS: [usize; 33] = [
    0, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15, 17, 18, 19, 21, 22, 23, 25, 26, 27, 29, 30, 31, 33,
    34, 35, 37, 38, 39, 41, 42, 43,
];

fn from_char(c: u8) -> LinkResult<u8> {
    // 0 - 9
    if c < b'0' {
        Err(ErrorKind::Deserialization)?;
    }
    if c <= b'9' {
        return Ok(c - b'0');
    }

    // A-Z
    if c < b'A' {
        Err(ErrorKind::Deserialization)?;
    }
    if c <= b'Z' {
        return Ok(c - b'A' + 10);
    }

    // a-z
    if c < b'a' {
        Err(ErrorKind::Deserialization)?;
    }
    if c <= b'z' {
        return Ok(c - b'a' + 36);
    }

    // c > 'z'
    Err(ErrorKind::Deserialization)?
}

fn get_block_len(l: usize) -> LinkResult<usize> {
    for (i, count) in TAIL_CHARS.iter().enumerate(){
        if *count == l {
            return Ok(i);
        }
        if *count > l {
            Err(ErrorKind::Deserialization)?;
        }
    }
    Err(ErrorKind::Deserialization)?
}

pub fn base62_encode(data: &[u8]) -> String {
    let mut ret = String::with_capacity(1);
    let mut chunks = data.chunks_exact(32);
    for block in &mut chunks {
        encode_chunk(block, 43, &mut ret);
    }

    let tail = chunks.remainder();
    if !tail.is_empty() {
        encode_chunk(tail, TAIL_CHARS[tail.len()], &mut ret);
    }

    ret
}

fn encode_chunk(bytes: &[u8], width: usize, out: &mut String) {
    debug_assert!(bytes.len() <= 32, "encode chunk is designed to take a <= 32 byte long slice");
    
    let mut num = [0u64; 4];

    let mut buf = [0u8; 32];
    buf[32-bytes.len()..].copy_from_slice(bytes); 
    for (i, col) in num.iter_mut().enumerate(){
        *col = u64::from_be_bytes(buf[i * 8..(i + 1) * 8].try_into().unwrap());
    }

    let mut buf = [b'0'; 43];
    for slot in buf[..width].iter_mut().rev() {
        *slot = ALPHABET[take_digit(&mut num)];
    }
    debug_assert!(num == [0u64; 4], "Num not entirely consumed, check TAIL_CHARS for correctness");

    out.push_str(str::from_utf8(&buf[..width]).unwrap());
}

fn take_digit(num: &mut [u64; 4]) -> usize {
    let mut rem: u128 = 0;

    for col in num.iter_mut() {
        let acc = (rem << 64) | *col as u128;
        *col = (acc / 62) as u64;
        rem = acc % 62;
    }

    rem as usize
}

pub fn base62_decode(data: &[u8]) -> LinkResult<Vec<u8>> {
    let mut ret = Vec::new();
    let mut chunks = data.chunks_exact(43);

    for chunk in &mut chunks {
        decode_chunk(chunk, &mut ret)?;
    }

    let tail = chunks.remainder();
    if !tail.is_empty() {
        decode_chunk(tail, &mut ret)?;
    }

    Ok(ret)
}

fn decode_chunk(data: &[u8], out: &mut Vec<u8>) -> LinkResult {
    let chunk_len = get_block_len(data.len())?;
    let mut num = [0u64; 4];

    for c in data {
        acc_digit(&mut num, from_char(*c)?)?;
    }
    
    let mut buf = [0u8; 32];

    for (i, col) in num.iter().enumerate(){
        buf[i * 8..(i+1)*8].copy_from_slice(&col.to_be_bytes());
    }

    // Reject inputs that produce non zero bytes outside the required length
    if buf[..32 - chunk_len].iter().any(|b| *b != 0) {
        Err(ErrorKind::Deserialization)?;
    }

    out.extend_from_slice(&buf[32-chunk_len..]);

    Ok(())
}

fn acc_digit(num: &mut [u64; 4], mut offset: u8) -> LinkResult {
    let mut carry = 0;
    for col in num.iter_mut().rev() {
        let res = *col as u128 * 62 + carry;
        *col = (res & u64::MAX as u128) as u64;
        carry = res >> 64;
    }
    if carry != 0 {
        // Ensure no left over data
        Err(ErrorKind::Deserialization)?;
    }

    let mut carry = false;
    for col in num.iter_mut().rev() {
        let (ret, c) = col.carrying_add(offset as u64, carry);
        *col = ret;
        offset = 0;
        carry = c;
    }

    if carry {
        // Ensure no left over data
        Err(ErrorKind::Deserialization)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn char_round_trip_low() {
        let msg = 5u8;

        let encoded = ALPHABET[msg as usize];
        println!("{}", encoded);

        let decoded = from_char(encoded).unwrap();

        assert_eq!(msg, decoded);
    }

    #[test]
    fn char_round_trip_med() {
        let msg = 20u8;

        let encoded = ALPHABET[msg as usize];
        println!("{}", encoded);

        let decoded = from_char(encoded).unwrap();

        assert_eq!(msg, decoded);
    }

    #[test]
    fn char_round_trip_high() {
        let msg = 60u8;

        let encoded = ALPHABET[msg as usize];
        println!("{}", encoded);

        let decoded = from_char(encoded).unwrap();

        assert_eq!(msg, decoded);
    }

    #[test]
    fn simple_round_trip() {
        let msg = b"test message";

        let encoded = base62_encode(msg);
        println!("{}", encoded);

        let decoded = base62_decode(encoded.as_bytes()).unwrap();

        assert_eq!(*msg, *decoded);
    }

    #[test]
    fn exact_round_trip() {
        let msg: &[u8; 32] = b"The quick brown fox jumps over t";

        let encoded = base62_encode(msg);
        println!("{}", encoded);

        let decoded = base62_decode(encoded.as_bytes()).unwrap();

        assert_eq!(*msg, *decoded);
    }

    #[test]
    fn long_round_trip() {
        let msg = b"The quick brown fox jumps over the lazy dog";

        let encoded = base62_encode(msg);
        println!("{}", encoded);

        let decoded = base62_decode(encoded.as_bytes()).unwrap();

        assert_eq!(*msg, *decoded);
    }

    #[test]
    fn known_answer() {
        // Pins the alphabet and block format. If this test breaks,
        // previously issued encodings will no longer decode.
        let encoded = base62_encode(b"test message");
        assert_eq!(encoded, "0kqeq9uPPAdXCr6lp");

        let decoded = base62_decode(encoded.as_bytes()).unwrap();
        assert_eq!(*b"test message", *decoded);
    }

    #[test]
    fn empty_round_trip() {
        assert_eq!(base62_encode(b""), "");
        assert_eq!(base62_decode(b"").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn all_lengths_round_trip() {
        // Every tail width once, plus the multi-block path (two blocks + tail)
        let data: Vec<u8> = (0..96u8).map(|i| i.wrapping_mul(37).wrapping_add(1)).collect();

        for len in 0..=data.len() {
            let msg = &data[..len];

            let encoded = base62_encode(msg);
            let expected_chars = (len / 32) * 43 + TAIL_CHARS[len % 32];
            assert_eq!(encoded.len(), expected_chars, "encoded width for {len} bytes");

            let decoded = base62_decode(encoded.as_bytes()).unwrap();
            assert_eq!(*msg, *decoded, "round trip for {len} bytes");
        }
    }

    #[test]
    fn leading_zeros_round_trip() {
        let msg = [0u8, 0, 0, 5, 10, 15];
        let decoded = base62_decode(base62_encode(&msg).as_bytes()).unwrap();
        assert_eq!(msg, *decoded);

        // all zeros, spanning a block boundary
        let msg = [0u8; 40];
        let decoded = base62_decode(base62_encode(&msg).as_bytes()).unwrap();
        assert_eq!(msg, *decoded);
    }

    #[test]
    fn max_value_round_trip() {
        // largest full-block value
        let msg = [0xFFu8; 32];
        let decoded = base62_decode(base62_encode(&msg).as_bytes()).unwrap();
        assert_eq!(msg, *decoded);

        // largest tail value
        let msg = [0xFFu8; 12];
        let decoded = base62_decode(base62_encode(&msg).as_bytes()).unwrap();
        assert_eq!(msg, *decoded);
    }

    #[test]
    fn reject_invalid_char() {
        assert!(base62_decode(b"0kqeq9uPPAdXCr6l!").is_err());
        assert!(base62_decode(b"0kqeq9uPPAdXC 6lp").is_err());
        // base64url's extra characters must not be accepted
        assert!(base62_decode(b"0kqeq9uPPAdXCr6l-").is_err());
        assert!(base62_decode(b"0kqeq9uPPAdXCr6l_").is_err());
    }

    #[test]
    fn reject_invalid_length() {
        // char counts that no byte length produces: 1, 4, 8, ...
        assert!(base62_decode(b"z").is_err());
        assert!(base62_decode(b"0000").is_err());
        assert!(base62_decode(b"00000000").is_err());
        // a full block followed by an invalid tail
        assert!(base62_decode(&b"0".repeat(44)).is_err());
    }

    #[test]
    fn reject_block_overflow() {
        // 62^43 > 2^256: the max 43-char string encodes no 32-byte block
        assert!(base62_decode(&b"z".repeat(43)).is_err());
    }

    #[test]
    fn reject_tail_overflow() {
        // 2 chars decode to 1 byte, but 62^2 - 1 = 3843 > 255
        assert!(base62_decode(b"zz").is_err());
        // 17 chars decode to 12 bytes, but 62^17 > 2^96
        assert!(base62_decode(&b"z".repeat(17)).is_err());
    }
}
