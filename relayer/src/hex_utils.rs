use primitive_types::{H160, H256};

pub fn decode_h256(hex: &str) -> anyhow::Result<H256> {
    let data: [u8; 32] = decode_byte_array(hex)?;
    Ok(data.into())
}

pub fn decode_h160(hex: &str) -> anyhow::Result<H160> {
    let data: [u8; 20] = decode_byte_array(hex)?;
    Ok(data.into())
}

pub fn decode_byte_array<const LEN: usize>(hex: &str) -> anyhow::Result<[u8; LEN]> {
    decode_byte_vec(hex)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid length"))
}

pub fn decode_byte_vec(hex: &str) -> anyhow::Result<Vec<u8>> {
    let address = if &hex[..2] == "0x" { &hex[2..] } else { hex };
    Ok(hex::decode(address)?)
}
