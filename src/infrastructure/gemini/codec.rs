use anyhow::{Result, bail};

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
