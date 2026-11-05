use std::io::{self, Read};

#[derive(Debug)]
pub(crate) struct BoundedOutput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) exceeded: bool,
}

pub(crate) fn collect_bounded(mut reader: impl Read, limit: usize) -> io::Result<BoundedOutput> {
    let mut bytes = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0_u8; 8_192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if !exceeded {
            let kept = limit.saturating_sub(bytes.len()).min(read);
            bytes.extend(buffer.iter().take(kept).copied());
            exceeded = kept < read;
        }
    }
    Ok(BoundedOutput { bytes, exceeded })
}
