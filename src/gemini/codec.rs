use anyhow::{Result, bail};

/// Encode one raw payload as unwrapped base64 text.
pub(super) fn encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut value = String::with_capacity(data.len().div_ceil(3) * 4);
    for block in data.chunks(3) {
        let first = block[0];
        let second = block.get(1).copied().unwrap_or(0);
        let third = block.get(2).copied().unwrap_or(0);
        value.push(char::from(TABLE[usize::from(first >> 2)]));
        value.push(char::from(
            TABLE[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        if block.len() > 1 {
            value.push(char::from(
                TABLE[usize::from(((second & 0x0f) << 2) | (third >> 6))],
            ));
        } else {
            value.push('=');
        }
        if block.len() > 2 {
            value.push(char::from(TABLE[usize::from(third & 0x3f)]));
        } else {
            value.push('=');
        }
    }
    value
}

/// Decode one base64 payload into raw bytes.
pub(super) fn decode(data: &str) -> Result<Vec<u8>> {
    let mut value = Vec::new();
    let mut block = Vec::new();
    for item in data.chars().filter(|item| !item.is_whitespace()) {
        if item == '=' {
            block.push(64);
        } else {
            block.push(code(item)?);
        }
        if block.len() == 4 {
            append(&mut value, &block)?;
            block.clear();
        }
    }
    if !block.is_empty() {
        bail!("Malformed base64 response payload");
    }
    Ok(value)
}

fn code(item: char) -> Result<u8> {
    match item {
        'A'..='Z' => Ok((item as u8) - b'A'),
        'a'..='z' => Ok((item as u8) - b'a' + 26),
        '0'..='9' => Ok((item as u8) - b'0' + 52),
        '+' => Ok(62),
        '/' => Ok(63),
        _ => bail!("Malformed base64 response payload"),
    }
}

fn append(value: &mut Vec<u8>, block: &[u8]) -> Result<()> {
    if block.len() != 4 {
        bail!("Malformed base64 response payload");
    }
    let first = (block[0] << 2) | (block[1] >> 4);
    value.push(first);
    if block[2] == 64 {
        return Ok(());
    }
    let second = ((block[1] & 0x0f) << 4) | (block[2] >> 2);
    value.push(second);
    if block[3] == 64 {
        return Ok(());
    }
    let third = ((block[2] & 0x03) << 6) | block[3];
    value.push(third);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn encoder_round_trips_irregular_binary_payload() {
        let source = [0, 1, 2, 3, 254, 255, 17];
        assert_eq!(
            decode(encode(&source).as_str()).expect("encoded data must decode"),
            source,
            "base64 encoder corrupted the multimodal request payload"
        );
    }
}
